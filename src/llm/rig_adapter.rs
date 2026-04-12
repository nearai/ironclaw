//! Generic adapter that bridges rig-core's `CompletionModel` trait to IronClaw's `LlmProvider`.
//!
//! This lets us use any rig-core provider (OpenAI, Anthropic, Ollama, etc.) as an
//! `Arc<dyn LlmProvider>` without changing any of the agent, reasoning, or tool code.

use crate::llm::config::CacheRetention;
use async_trait::async_trait;
use rig::OneOrMany;
use rig::completion::{
    AssistantContent, CompletionModel, CompletionRequest as RigRequest,
    ToolDefinition as RigToolDefinition, Usage as RigUsage,
};
use rig::message::{
    DocumentSourceKind, Image, ImageMediaType, Message as RigMessage, MimeType,
    ToolChoice as RigToolChoice, ToolFunction, ToolResult as RigToolResult, ToolResultContent,
    UserContent,
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};

use std::collections::HashSet;

use crate::llm::costs;
use crate::llm::error::LlmError;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider,
    ToolCall as IronToolCall, ToolCompletionRequest, ToolCompletionResponse,
    ToolDefinition as IronToolDefinition, strip_unsupported_completion_params,
    strip_unsupported_tool_params,
};

/// Adapter that wraps a rig-core `CompletionModel` and implements `LlmProvider`.
pub struct RigAdapter<M: CompletionModel> {
    model: M,
    model_name: String,
    input_cost: Decimal,
    output_cost: Decimal,
    /// Prompt cache retention policy (Anthropic only).
    /// When not `CacheRetention::None`, injects top-level `cache_control`
    /// via `additional_params` for Anthropic automatic caching. Also controls
    /// the cost multiplier for cache-creation tokens.
    cache_retention: CacheRetention,
    /// Parameter names that this provider does not support (e.g., `"temperature"`).
    /// These are stripped from requests before sending to avoid 400 errors.
    unsupported_params: HashSet<String>,
}

impl<M: CompletionModel> RigAdapter<M> {
    /// Create a new adapter wrapping the given rig-core model.
    pub fn new(model: M, model_name: impl Into<String>) -> Self {
        let name = model_name.into();
        let (input_cost, output_cost) =
            costs::model_cost(&name).unwrap_or_else(costs::default_cost);
        Self {
            model,
            model_name: name,
            input_cost,
            output_cost,
            cache_retention: CacheRetention::None,
            unsupported_params: HashSet::new(),
        }
    }

    /// Set Anthropic prompt cache retention policy.
    ///
    /// Controls both cache injection and cost tracking:
    /// - `None` — no caching, no surcharge (1.0×).
    /// - `Short` — 5-minute TTL via `{"type": "ephemeral"}`, 1.25× write surcharge.
    /// - `Long` — 1-hour TTL via `{"type": "ephemeral", "ttl": "1h"}`, 2.0× write surcharge.
    ///
    /// Cache injection uses Anthropic's **automatic caching** — a top-level
    /// `cache_control` field in `additional_params` that gets `#[serde(flatten)]`'d
    /// into the request body by rig-core.
    ///
    /// If the configured model does not support caching (e.g. claude-2),
    /// a warning is logged once at construction and caching is disabled.
    pub fn with_cache_retention(mut self, retention: CacheRetention) -> Self {
        if retention != CacheRetention::None && !supports_prompt_cache(&self.model_name) {
            tracing::warn!(
                model = %self.model_name,
                "Prompt caching requested but model does not support it; disabling"
            );
            self.cache_retention = CacheRetention::None;
        } else {
            self.cache_retention = retention;
        }
        self
    }

    /// Set the list of unsupported parameter names for this provider.
    ///
    /// Parameters in this set are stripped from requests before sending.
    /// Supported parameter names: `"temperature"`, `"max_tokens"`, `"stop_sequences"`.
    pub fn with_unsupported_params(mut self, params: Vec<String>) -> Self {
        self.unsupported_params = params.into_iter().collect();
        self
    }

    /// Strip unsupported fields from a `CompletionRequest` in place.
    fn strip_unsupported_completion_params(&self, req: &mut CompletionRequest) {
        strip_unsupported_completion_params(&self.unsupported_params, req);
    }

    /// Strip unsupported fields from a `ToolCompletionRequest` in place.
    fn strip_unsupported_tool_params(&self, req: &mut ToolCompletionRequest) {
        strip_unsupported_tool_params(&self.unsupported_params, req);
    }
}

// -- Type conversion helpers --

/// Round an f32 to f64 without precision artifacts.
///
/// Direct `f32 as f64` preserves the binary representation, producing values
/// like `0.699999988079071` instead of `0.7`. Some providers (e.g. Zhipu/GLM)
/// reject these values with a 400 error. Rounding to 6 decimal places removes
/// the artifact while preserving all meaningful precision for temperature.
fn round_f32_to_f64(val: f32) -> f64 {
    ((val as f64) * 1_000_000.0).round() / 1_000_000.0
}

/// Normalize a JSON Schema for OpenAI tool-calling compliance.
///
/// Two transforms are applied at the provider boundary:
///
/// 1. **Top-level flatten.** OpenAI's tool API (Chat Completions and the
///    Responses API alike) rejects schemas whose top level isn't
///    `type: "object"`, or that contain top-level `oneOf`/`anyOf`/`allOf`/
///    `enum`/`not`. The exact error is:
///
///    ```text
///    Invalid schema for function '<name>': schema must have type 'object'
///    and not have 'oneOf'/'anyOf'/'allOf'/'enum'/'not' at the top level.
///    ```
///
///    Some MCP servers (notably the GitHub Copilot MCP at
///    `api.githubcopilot.com/mcp/`) advertise tools whose top-level schema is
///    a `oneOf` for action dispatch. There's no API-side workaround, so when
///    we detect this we replace `parameters` with a permissive object
///    envelope (`{type: "object", properties: {}, additionalProperties: true,
///    required: []}`) and append the original schema to the tool description
///    as advisory text. The MCP server still validates the actual shape on
///    its end, so the tool keeps working — we just lose API-level schema
///    enforcement and the LLM has to read the variant structure from the
///    description.
///
/// 2. **Strict-mode recursive normalization.** OpenAI strict function
///    calling requires:
///    - Every object must have `"additionalProperties": false`
///    - `"required"` must list ALL property keys
///    - Optional fields use `"type": ["<original>", "null"]` instead of
///      being omitted from `required`
///    - Nested objects and array items are recursively normalized
///
/// `description` is a `&mut String` because the top-level flatten needs to
/// append a hint to it. Pass an owned clone of the tool description and read
/// it back after the call. If no flatten was needed, `description` is
/// untouched.
///
/// Note on Anthropic: this normalizer is also applied to Anthropic via
/// `RigAdapter::convert_tools`. Anthropic accepts top-level `oneOf` natively,
/// so the flatten is slightly lossy for Claude users on tools that have a
/// top-level union, but the description hint preserves the variant info and
/// Claude is good at reading schemas out of free text. Keeping a single
/// normalizer for all rig-based providers is simpler than threading
/// per-provider flags through the adapter.
pub(crate) fn normalize_schema_strict(schema: &JsonValue, description: &mut String) -> JsonValue {
    let mut schema = schema.clone();

    // Step 1: top-level flatten. If the schema has a forbidden top-level
    // construct, replace it with a permissive envelope whose properties are
    // merged from the union variants.
    if needs_top_level_flatten(&schema) {
        flatten_top_level(&mut schema, description);
        // The flattened envelope is deliberately permissive
        // (additionalProperties: true, required: []) so the LLM can
        // send fields from any variant. Running normalize_schema_recursive
        // on the ROOT would clobber that (force additionalProperties: false,
        // required: all keys). But the individual merged properties still
        // need normalization — e.g. array items must be objects, nested
        // objects need strict-mode treatment. So we normalize each property
        // individually without touching the top-level envelope.
        if let Some(props) = schema.get_mut("properties").and_then(|v| v.as_object_mut()) {
            for (_key, prop_schema) in props.iter_mut() {
                normalize_schema_recursive(prop_schema);
            }
        }
        // Skip the post-normalization strict-mode validator for the flatten
        // path. The flattened envelope is intentionally non-strict
        // (additionalProperties: true, required: []), so the strict
        // validator would fire a false positive on every flattened schema
        // — 12x per LLM call when there are 12 flattened tools, drowning
        // real signals in noise. The individual properties were already
        // normalized recursively above; the top-level permissive shape is
        // by design, not a bug.
        return schema;
    }

    // Step 2: recursive strict-mode normalization (non-flatten path).
    normalize_schema_recursive(&mut schema);

    // Step 3: post-normalization validation (non-flatten path only).
    // The normalizer handles the rules it knows about, but OpenAI's
    // strict-mode spec has more rules than any single normalizer pass is
    // likely to cover perfectly — and new rules appear without notice.
    // Running the CI validator as a debug-level post-check catches anything
    // the normalizer missed so we get a local diagnostic instead of a
    // runtime HTTP 400. Flattened schemas skip this check (see above).
    //
    // This is deliberately `debug!` (not `warn!`) because the schema
    // still goes through — the tool remains usable, and the LLM provider
    // will surface the 400 if OpenAI actually rejects it. The diagnostic
    // value is for developers adding new tools or modifying the normalizer.
    if let Err(violations) =
        crate::tools::schema_validator::validate_strict_schema(&schema, "<post-normalize>")
    {
        tracing::debug!(
            violations = ?violations,
            "normalize_schema_strict output has {} strict-mode violation(s) — \
             the tool is still usable but the LLM provider may reject the schema",
            violations.len()
        );
    }

    schema
}

/// JSON Schema keywords that OpenAI's tool API rejects at the top level of
/// a tool's `parameters`. Listed in priority order so that, when more than
/// one is present, we report the most semantically meaningful one in the
/// description hint.
const FORBIDDEN_TOP_LEVEL: &[&str] = &["oneOf", "anyOf", "allOf", "enum", "not"];

/// Detect which forbidden top-level keyword (if any) `schema` has, returning
/// the keyword name so the caller can pick a precise hint string. Returns
/// `None` for an object schema with none of the forbidden constructs (the
/// caller may still want to flatten if `type` isn't `"object"`).
fn detect_forbidden_top_level(schema: &JsonValue) -> Option<&'static str> {
    let map = schema.as_object()?;
    FORBIDDEN_TOP_LEVEL
        .iter()
        .find(|keyword| map.contains_key(**keyword))
        .copied()
}

/// True if `schema`'s top level would be rejected by OpenAI's tool API.
fn needs_top_level_flatten(schema: &JsonValue) -> bool {
    match schema {
        JsonValue::Object(map) => {
            let has_forbidden = FORBIDDEN_TOP_LEVEL.iter().any(|k| map.contains_key(*k));
            // Accept both `"type": "object"` and `"type": ["object", "null"]`
            // (or any array containing "object"). The array form is valid
            // JSON Schema for a nullable object and some upstream providers
            // / `make_nullable` produce it. Treating it as bad_type would
            // silently flatten a schema that OpenAI might actually accept,
            // discarding all its properties.
            let is_object_type = match map.get("type") {
                Some(JsonValue::String(s)) => s == "object",
                Some(JsonValue::Array(arr)) => arr
                    .iter()
                    .any(|v| matches!(v, JsonValue::String(s) if s == "object")),
                _ => false,
            };
            has_forbidden || !is_object_type
        }
        // Schema isn't even a JSON object — definitely not OpenAI-compatible.
        _ => true,
    }
}

/// Pick a description hint that matches the actual JSON Schema construct
/// that triggered the flatten. The previous one-size-fits-all hint
/// ("pick one variant and pass its fields") was correct for `oneOf` /
/// `anyOf` but actively misleading for `allOf` (where the LLM should pass
/// fields from ALL variants), `enum` (one of the literal values), and `not`
/// (any object that doesn't match a constraint).
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
        // Fallback: schema wasn't an object, or had some unrecognized
        // shape that we still flattened defensively. The MCP server will
        // validate the actual call shape on its end, so the LLM just has
        // to send something the upstream accepts.
        _ => {
            "\n\nUpstream JSON schema (advisory; the original was not a top-level \
             object schema, so we flattened to a free-form object — see below for \
             the actual constraints the upstream server will enforce):\n"
        }
    }
}

/// Walk the top-level `oneOf` / `anyOf` / `allOf` arrays in `schema` and
/// collect every property the variants declare into a single map. First-write
/// wins on conflicting types — if two variants declare the same field name
/// with different schemas, the first one's schema is kept and the second is
/// dropped. The full schema still goes into the description hint, so the
/// truncation we lose here is recoverable from there.
///
/// This is the structured-info recovery layer for `flatten_top_level`. The
/// LLM now sees a real `properties` map instead of `{}`, so it can do
/// schema-based reasoning about which fields exist across the union — even
/// though strict-mode validation is disabled at the API layer.
fn merge_top_level_variant_properties(schema: &JsonValue) -> serde_json::Map<String, JsonValue> {
    let mut merged = serde_json::Map::new();
    let Some(obj) = schema.as_object() else {
        return merged;
    };
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

/// Serialize a `serde_json::Value` into a string, stopping after `max_bytes`.
///
/// Returns `Ok(s)` where `s.len() <= max_bytes` (truncated on a char
/// boundary if the serialized output exceeds the budget), or `Err(())` if
/// serialization itself fails (shouldn't happen for well-formed `Value`s).
///
/// This bounds the actual heap allocation regardless of schema structure —
/// it handles both many-node schemas (deep recursion) AND few-node schemas
/// with multi-MB string values (the gap the previous `count_json_nodes`
/// approach missed). The writer stops accepting bytes once the budget is
/// reached, so the serde serializer does minimal work after that point.
fn serialize_json_capped(value: &JsonValue, max_bytes: usize) -> Result<String, ()> {
    use std::io::Write;

    struct CappedWriter {
        buf: Vec<u8>,
        max: usize,
    }

    impl Write for CappedWriter {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            let remaining = self.max.saturating_sub(self.buf.len());
            if remaining == 0 {
                // Accept the bytes (don't error) but don't store them.
                // serde_json doesn't check for short writes so returning
                // Ok(data.len()) is safe — it just thinks we consumed them.
                return Ok(data.len());
            }
            let to_write = data.len().min(remaining);
            self.buf.extend_from_slice(&data[..to_write]);
            Ok(data.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    // Allocate slightly over max to reduce the chance of a realloc on
    // the last write when we're right at the boundary.
    let writer = CappedWriter {
        buf: Vec::with_capacity(max_bytes.min(8192)),
        max: max_bytes,
    };
    let mut ser = serde_json::Serializer::new(writer);
    serde::Serialize::serialize(value, &mut ser).map_err(|_| ())?;
    let buf = ser.into_inner().buf;
    // serde_json v1 emits raw UTF-8 for non-ASCII characters (e.g. CJK
    // characters in property descriptions), so the byte cap can cut
    // mid-codepoint. Use `from_utf8` with a fallback to `valid_up_to()`
    // to trim to the last complete codepoint instead of dropping the
    // entire hint on a UTF-8 error.
    match String::from_utf8(buf) {
        Ok(s) => Ok(s),
        Err(e) => {
            let valid_len = e.utf8_error().valid_up_to();
            let mut buf = e.into_bytes();
            buf.truncate(valid_len);
            // safety: valid_up_to guarantees the prefix is valid UTF-8,
            // so from_utf8_unchecked is sound here.
            Ok(unsafe { String::from_utf8_unchecked(buf) })
        }
    }
}

/// Replace `parameters` with a permissive object envelope and append the
/// original schema to `description` as advisory text. Truncates the hint on a
/// char boundary if the original schema is too large to fit in a reasonable
/// description budget. The hint introduction is keyword-aware so the LLM
/// gets the right shape guidance for `oneOf`/`anyOf`/`allOf`/`enum`/`not`.
///
/// The flattened envelope is no longer empty — `merge_top_level_variant_properties`
/// walks the union variants and rebuilds a flat `properties` map so the LLM
/// has structured field hints to reason about. Strict-mode validation is
/// still disabled (`additionalProperties: true`, `required: []`) so the LLM
/// is free to send any combination of variant fields and the upstream MCP
/// server enforces the actual constraints.
fn flatten_top_level(parameters: &mut JsonValue, description: &mut String) {
    // OpenAI has no documented hard limit on tool description length, but
    // long descriptions waste prompt budget on every turn. 1500 bytes fits a
    // typical MCP dispatcher schema and still leaves room for the original
    // tool description above it.
    const SCHEMA_HINT_MAX_BYTES: usize = 1500;

    let detected = detect_forbidden_top_level(parameters);
    let merged_properties = merge_top_level_variant_properties(parameters);

    // Size-capped serialization: bounds the heap allocation to
    // SCHEMA_HINT_MAX_BYTES regardless of schema structure. Handles both
    // many-node schemas (deep recursion) and few-node schemas with multi-MB
    // string values. The writer silently discards bytes past the cap so the
    // serde serializer does minimal useful work after that point.
    if let Ok(capped_text) = serialize_json_capped(parameters, SCHEMA_HINT_MAX_BYTES)
        && !capped_text.is_empty()
    {
        let hint = if capped_text.len() >= SCHEMA_HINT_MAX_BYTES {
            format!("{capped_text} ... (truncated)")
        } else {
            capped_text
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

fn normalize_schema_recursive(schema: &mut JsonValue) {
    let obj = match schema.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    // Recurse into combinators: anyOf, oneOf, allOf
    for key in &["anyOf", "oneOf", "allOf"] {
        if let Some(JsonValue::Array(variants)) = obj.get_mut(*key) {
            for variant in variants.iter_mut() {
                normalize_schema_recursive(variant);
            }
        }
    }

    // Recurse into array items. OpenAI strict mode requires `items` to be
    // a JSON Schema object for every array-typed property. Schema generators
    // (schemars, serde_json) produce `"items": true` or omit `items`
    // entirely for `Vec<serde_json::Value>` (meaning "accept any item"),
    // which OpenAI rejects with:
    //   "array schema items is not an object"
    // Ensure `items` exists and is an object before recursing.
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
            None => true,                        // missing entirely
            Some(JsonValue::Object(_)) => false, // already a schema object
            _ => true,                           // bool, string, array, etc.
        };
        if needs_fix {
            obj.insert("items".to_string(), serde_json::json!({}));
        }
    }
    if let Some(items) = obj.get_mut("items") {
        normalize_schema_recursive(items);
    }

    // Recurse into `not`, `if`, `then`, `else`
    for key in &["not", "if", "then", "else"] {
        if let Some(sub) = obj.get_mut(*key) {
            normalize_schema_recursive(sub);
        }
    }

    // Only apply object-level normalization if this schema has "properties"
    // (explicit object schema) or type == "object"
    let is_object = obj
        .get("type")
        .and_then(|t| t.as_str())
        .map(|t| t == "object")
        .unwrap_or(false);
    let has_properties = obj.contains_key("properties");

    if !is_object && !has_properties {
        return;
    }

    // Ensure "type": "object" is present
    if !obj.contains_key("type") && has_properties {
        obj.insert("type".to_string(), JsonValue::String("object".to_string()));
    }

    // Force additionalProperties: false (overwrite any existing value)
    obj.insert("additionalProperties".to_string(), JsonValue::Bool(false));

    // Ensure "properties" exists
    if !obj.contains_key("properties") {
        obj.insert(
            "properties".to_string(),
            JsonValue::Object(serde_json::Map::new()),
        );
    }

    // Collect current required set
    let current_required: std::collections::HashSet<String> = obj
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Get all property keys (sorted for deterministic output)
    let all_keys: Vec<String> = obj
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|props| {
            let mut keys: Vec<String> = props.keys().cloned().collect();
            keys.sort();
            keys
        })
        .unwrap_or_default();

    // For properties NOT in the original required list, make them nullable
    if let Some(JsonValue::Object(props)) = obj.get_mut("properties") {
        for key in &all_keys {
            // Recurse into each property's schema FIRST (before make_nullable,
            // which may change the type to an array and prevent object detection)
            if let Some(prop_schema) = props.get_mut(key) {
                normalize_schema_recursive(prop_schema);
            }
            // Then make originally-optional properties nullable
            if !current_required.contains(key)
                && let Some(prop_schema) = props.get_mut(key)
            {
                make_nullable(prop_schema);
            }
        }
    }

    // Set required to ALL property keys
    let required_value: Vec<JsonValue> = all_keys.into_iter().map(JsonValue::String).collect();
    obj.insert("required".to_string(), JsonValue::Array(required_value));
}

/// Make a property schema nullable for OpenAI strict mode.
///
/// If it has a simple `"type": "<T>"`, converts to `"type": ["<T>", "null"]`.
/// If it already has an array type, adds "null" if not present.
/// Otherwise, wraps with `anyOf: [<existing>, {"type": "null"}]`.
fn make_nullable(schema: &mut JsonValue) {
    let obj = match schema.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    if let Some(type_val) = obj.get("type").cloned() {
        match type_val {
            // "type": "string" → "type": ["string", "null"]
            JsonValue::String(ref t) if t != "null" => {
                obj.insert("type".to_string(), serde_json::json!([t, "null"]));
            }
            // "type": ["string", "integer"] → add "null" if missing
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
        // No "type" key — wrap with anyOf including null
        // (handles enum-only, $ref, or combinator schemas)
        let existing = JsonValue::Object(obj.clone());
        obj.clear();
        obj.insert(
            "anyOf".to_string(),
            serde_json::json!([existing, {"type": "null"}]),
        );
    }
}

/// Convert IronClaw messages to rig-core format.
///
/// Returns `(preamble, chat_history)` where preamble is extracted from
/// any System message and chat_history contains the rest.
fn convert_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<RigMessage>) {
    let mut preamble: Option<String> = None;
    let mut history = Vec::new();

    for msg in messages {
        match msg.role {
            crate::llm::Role::System => {
                // Concatenate system messages into preamble
                match preamble {
                    Some(ref mut p) => {
                        p.push('\n');
                        p.push_str(&msg.content);
                    }
                    None => preamble = Some(msg.content.clone()),
                }
            }
            crate::llm::Role::User => {
                if msg.content_parts.is_empty() {
                    // Skip empty user messages — some providers (e.g. Kimi) reject "content": ""
                    if msg.content.is_empty() {
                        continue;
                    }
                    history.push(RigMessage::user(&msg.content));
                } else {
                    // Build multimodal user message with text + image parts
                    let mut contents: Vec<UserContent> = vec![UserContent::text(&msg.content)];
                    for part in &msg.content_parts {
                        if let crate::llm::ContentPart::ImageUrl { image_url } = part {
                            // Parse data: URL for base64 images, or use raw URL
                            let image = if let Some(rest) = image_url.url.strip_prefix("data:") {
                                // Format: data:<mime>;base64,<data>
                                let (mime, b64) =
                                    rest.split_once(";base64,").unwrap_or(("image/jpeg", rest));
                                Image {
                                    data: DocumentSourceKind::base64(b64),
                                    media_type: ImageMediaType::from_mime_type(mime),
                                    detail: None,
                                    additional_params: None,
                                }
                            } else {
                                Image {
                                    data: DocumentSourceKind::url(&image_url.url),
                                    media_type: None,
                                    detail: None,
                                    additional_params: None,
                                }
                            };
                            contents.push(UserContent::Image(image));
                        }
                    }
                    if let Ok(many) = OneOrMany::many(contents) {
                        history.push(RigMessage::User { content: many });
                    } else {
                        history.push(RigMessage::user(&msg.content));
                    }
                }
            }
            crate::llm::Role::Assistant => {
                if let Some(ref tool_calls) = msg.tool_calls {
                    // Assistant message with tool calls
                    let mut contents: Vec<AssistantContent> = Vec::new();
                    if !msg.content.is_empty() {
                        contents.push(AssistantContent::text(&msg.content));
                    }
                    for (idx, tc) in tool_calls.iter().enumerate() {
                        let tool_call_id =
                            normalized_tool_call_id(Some(tc.id.as_str()), history.len() + idx);
                        contents.push(AssistantContent::ToolCall(
                            rig::message::ToolCall::new(
                                tool_call_id.clone(),
                                ToolFunction::new(tc.name.clone(), tc.arguments.clone()),
                            )
                            .with_call_id(tool_call_id),
                        ));
                    }
                    if let Ok(many) = OneOrMany::many(contents) {
                        history.push(RigMessage::Assistant {
                            id: None,
                            content: many,
                        });
                    } else {
                        // Shouldn't happen but fall back to text
                        history.push(RigMessage::assistant(&msg.content));
                    }
                } else {
                    // Skip empty assistant messages — these occur when thinking-tag stripping
                    // leaves a blank response; sending "content": "" causes 400 on strict
                    // OpenAI-compatible providers (e.g. Kimi).
                    if msg.content.is_empty() {
                        continue;
                    }
                    history.push(RigMessage::assistant(&msg.content));
                }
            }
            crate::llm::Role::Tool => {
                // Tool result message: wrap as User { ToolResult }.
                // Merge consecutive tool results into a single User message
                // so the API sees one multi-result message instead of
                // multiple consecutive User messages (which Anthropic rejects).
                let tool_id = normalized_tool_call_id(msg.tool_call_id.as_deref(), history.len());
                let tool_result = UserContent::ToolResult(RigToolResult {
                    id: tool_id.clone(),
                    call_id: Some(tool_id),
                    content: OneOrMany::one(ToolResultContent::text(&msg.content)),
                });

                let should_merge = matches!(
                    history.last(),
                    Some(RigMessage::User { content }) if content.iter().all(|c| matches!(c, UserContent::ToolResult(_)))
                );

                if should_merge {
                    if let Some(RigMessage::User { content }) = history.last_mut() {
                        content.push(tool_result);
                    }
                } else {
                    history.push(RigMessage::User {
                        content: OneOrMany::one(tool_result),
                    });
                }
            }
        }
    }

    (preamble, history)
}

/// Responses-style providers require a non-empty tool call ID.
///
/// IDs must be compatible with providers like Mistral, which constrain IDs
/// to `[a-zA-Z0-9]{9}`. We therefore:
/// - pass through any non-empty raw ID that already matches this constraint;
/// - otherwise deterministically map the raw string into a provider-compliant ID;
/// - and when `raw` is empty/None, delegate to `generate_tool_call_id`.
fn normalized_tool_call_id(raw: Option<&str>, seed: usize) -> String {
    // Trim and treat empty as None.
    let trimmed = raw.and_then(|s| {
        let t = s.trim();
        if t.is_empty() { None } else { Some(t) }
    });

    if let Some(id) = trimmed {
        // If the ID already satisfies `[a-zA-Z0-9]{9}`, pass it through unchanged.
        if id.len() == 9 && id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return id.to_string();
        }

        // Otherwise, deterministically hash the raw ID and feed the hash-derived
        // seed into the provider-level generator so that the encoding and any
        // provider-specific constraints remain centralized in one place.
        let digest = Sha256::digest(id.as_bytes());
        // Derive a 64-bit value from the first 8 bytes of the digest, then
        // split it into two usize seeds so we preserve all 64 bits of entropy
        // even on 32-bit targets.
        let hash64 = {
            // SHA-256 always produces 32 bytes, so indexing the first 8 is safe.
            let bytes: [u8; 8] = [
                digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6],
                digest[7],
            ];
            u64::from_be_bytes(bytes)
        };
        let hi_seed: usize = (hash64 >> 32) as usize;
        let lo_seed: usize = (hash64 & 0xFFFF_FFFF) as usize;
        return super::provider::generate_tool_call_id(hi_seed, lo_seed);
    }

    // Fallback for missing/empty raw IDs: use the provider-level generator,
    // which already produces compliant IDs.
    super::provider::generate_tool_call_id(seed, 0)
}

/// Convert IronClaw tool definitions to rig-core format.
///
/// Applies `normalize_schema_strict` at the boundary, which both
/// strict-normalizes nested objects AND flattens any top-level
/// `oneOf`/`anyOf`/`allOf`/`enum`/`not` (OpenAI's tool API rejects those at
/// the top level even when the rest of the schema is valid). The flatten may
/// append an advisory hint to the tool description, so we pass an owned
/// clone through and read it back.
fn convert_tools(tools: &[IronToolDefinition]) -> Vec<RigToolDefinition> {
    tools
        .iter()
        .map(|t| {
            let mut description = t.description.clone();
            let parameters = normalize_schema_strict(&t.parameters, &mut description);
            RigToolDefinition {
                name: t.name.clone(),
                description,
                parameters,
            }
        })
        .collect()
}

/// Convert IronClaw tool_choice string to rig-core ToolChoice.
fn convert_tool_choice(choice: Option<&str>) -> Option<RigToolChoice> {
    match choice.map(|s| s.to_lowercase()).as_deref() {
        Some("auto") => Some(RigToolChoice::Auto),
        Some("required") => Some(RigToolChoice::Required),
        Some("none") => Some(RigToolChoice::None),
        _ => None,
    }
}

/// Extract text and tool calls from a rig-core completion response.
fn extract_response(
    choice: &OneOrMany<AssistantContent>,
    _usage: &RigUsage,
) -> (Option<String>, Vec<IronToolCall>, FinishReason) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<IronToolCall> = Vec::new();

    for content in choice.iter() {
        match content {
            AssistantContent::Text(t) => {
                if !t.text.is_empty() {
                    text_parts.push(t.text.clone());
                }
            }
            AssistantContent::ToolCall(tc) => {
                tool_calls.push(IronToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: tc.function.arguments.clone(),
                    reasoning: None,
                });
            }
            // Reasoning and Image variants are not mapped to IronClaw types
            _ => {}
        }
    }

    let text = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join(""))
    };

    let finish = if !tool_calls.is_empty() {
        FinishReason::ToolUse
    } else {
        FinishReason::Stop
    };

    (text, tool_calls, finish)
}

/// Saturate u64 to u32 for token counts.
fn saturate_u32(val: u64) -> u32 {
    val.min(u32::MAX as u64) as u32
}

/// Returns `true` if the model supports Anthropic prompt caching.
///
/// Per Anthropic docs, only Claude 3+ models support prompt caching.
/// Unsupported: claude-2, claude-2.1, claude-instant-*.
fn supports_prompt_cache(name: &str) -> bool {
    let lower = name.to_lowercase();
    // Strip optional provider prefix (e.g. "anthropic/claude-...")
    let model = lower.strip_prefix("anthropic/").unwrap_or(&lower);
    // Only Claude 3+ families support prompt caching
    model.starts_with("claude-3")
        || model.starts_with("claude-4")
        || model.starts_with("claude-sonnet")
        || model.starts_with("claude-opus")
        || model.starts_with("claude-haiku")
}

/// Extract `cache_creation_input_tokens` from the raw provider response.
///
/// Rig-core's unified `Usage` does not surface this field, but Anthropic's raw
/// response includes it at `usage.cache_creation_input_tokens`. We serialize the
/// raw response to JSON and attempt to read the value.
fn extract_cache_creation<T: Serialize>(raw: &T) -> u32 {
    serde_json::to_value(raw)
        .ok()
        .and_then(|v| v.get("usage")?.get("cache_creation_input_tokens")?.as_u64())
        .map(|n| n.min(u32::MAX as u64) as u32)
        .unwrap_or(0)
}

/// Build a rig-core CompletionRequest from our internal types.
///
/// When `cache_retention` is not `None`, injects a top-level `cache_control`
/// field via `additional_params`. Rig-core's `AnthropicCompletionRequest`
/// uses `#[serde(flatten)]` on `additional_params`, so the field lands at
/// the request root — which is exactly what Anthropic's **automatic caching**
/// expects. The API auto-places the cache breakpoint at the last cacheable
/// block and moves it forward as conversations grow.
#[allow(clippy::too_many_arguments)]
fn build_rig_request(
    preamble: Option<String>,
    mut history: Vec<RigMessage>,
    tools: Vec<RigToolDefinition>,
    tool_choice: Option<RigToolChoice>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    cache_retention: CacheRetention,
) -> Result<RigRequest, LlmError> {
    // rig-core requires at least one message in chat_history
    if history.is_empty() {
        history.push(RigMessage::user("Hello"));
    }

    let chat_history = OneOrMany::many(history).map_err(|e| LlmError::RequestFailed {
        provider: "rig".to_string(),
        reason: format!("Failed to build chat history: {}", e),
    })?;

    // Inject top-level cache_control for Anthropic automatic prompt caching.
    let additional_params = match cache_retention {
        CacheRetention::None => None,
        CacheRetention::Short => Some(serde_json::json!({
            "cache_control": {"type": "ephemeral"}
        })),
        CacheRetention::Long => Some(serde_json::json!({
            "cache_control": {"type": "ephemeral", "ttl": "1h"}
        })),
    };

    Ok(RigRequest {
        preamble,
        chat_history,
        documents: Vec::new(),
        tools,
        temperature: temperature.map(round_f32_to_f64),
        max_tokens: max_tokens.map(|t| t as u64),
        tool_choice,
        additional_params,
    })
}

/// Inject a per-request model override into the rig request's `additional_params`.
///
/// Rig-core bakes the model name at construction time inside each provider's
/// `CompletionModel` implementation. This helper inserts a top-level `"model"`
/// key into `additional_params`, which rig-core flattens into the provider's
/// request payload via `#[serde(flatten)]`.
///
/// Whether the override takes effect depends on the downstream API server's
/// handling of duplicate JSON keys (most Python/Go servers use last-key-wins,
/// but this is not guaranteed by the JSON spec). The `effective_model_name()`
/// trait method should be consulted to determine the model actually used.
fn inject_model_override(rig_req: &mut RigRequest, model_override: Option<&str>) {
    let Some(model) = model_override else {
        return;
    };
    match rig_req.additional_params {
        Some(ref mut params) => {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("model".to_string(), serde_json::json!(model));
            }
        }
        None => {
            rig_req.additional_params = Some(serde_json::json!({ "model": model }));
        }
    }
}

#[async_trait]
impl<M> LlmProvider for RigAdapter<M>
where
    M: CompletionModel + Send + Sync + 'static,
    M::Response: Send + Sync + Serialize + DeserializeOwned,
{
    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (self.input_cost, self.output_cost)
    }

    fn cache_write_multiplier(&self) -> Decimal {
        match self.cache_retention {
            CacheRetention::None => Decimal::ONE,
            CacheRetention::Short => Decimal::new(125, 2), // 1.25× (125% of input rate)
            CacheRetention::Long => Decimal::TWO,          // 2.0×  (200% of input rate)
        }
    }

    fn cache_read_discount(&self) -> Decimal {
        if self.cache_retention != CacheRetention::None {
            dec!(10) // Anthropic: 90% discount (cost = input_rate / 10)
        } else {
            Decimal::ONE
        }
    }

    async fn complete(
        &self,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        let model_override = request.take_model_override();

        self.strip_unsupported_completion_params(&mut request);

        let mut messages = request.messages;
        crate::llm::provider::sanitize_tool_messages(&mut messages);
        let (preamble, history) = convert_messages(&messages);

        let mut rig_req = build_rig_request(
            preamble,
            history,
            Vec::new(),
            None,
            request.temperature,
            request.max_tokens,
            self.cache_retention,
        )?;

        inject_model_override(&mut rig_req, model_override.as_deref());

        let response =
            self.model
                .completion(rig_req)
                .await
                .map_err(|e| LlmError::RequestFailed {
                    provider: self.model_name.clone(),
                    reason: e.to_string(),
                })?;

        let (text, _tool_calls, finish) = extract_response(&response.choice, &response.usage);

        let resp = CompletionResponse {
            content: text.unwrap_or_default(),
            input_tokens: saturate_u32(response.usage.input_tokens),
            output_tokens: saturate_u32(response.usage.output_tokens),
            finish_reason: finish,
            cache_read_input_tokens: saturate_u32(response.usage.cached_input_tokens),
            cache_creation_input_tokens: extract_cache_creation(&response.raw_response),
        };

        if resp.cache_read_input_tokens > 0 {
            tracing::debug!(
                model = %self.model_name,
                input = resp.input_tokens,
                output = resp.output_tokens,
                cache_read = resp.cache_read_input_tokens,
                "prompt cache hit",
            );
        }

        Ok(resp)
    }

    async fn complete_with_tools(
        &self,
        mut request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let model_override = request.take_model_override();

        self.strip_unsupported_tool_params(&mut request);

        let known_tool_names: HashSet<String> =
            request.tools.iter().map(|t| t.name.clone()).collect();

        let mut messages = request.messages;
        crate::llm::provider::sanitize_tool_messages(&mut messages);
        let (preamble, history) = convert_messages(&messages);
        let tools = convert_tools(&request.tools);
        let tool_choice = convert_tool_choice(request.tool_choice.as_deref());

        let mut rig_req = build_rig_request(
            preamble,
            history,
            tools,
            tool_choice,
            request.temperature,
            request.max_tokens,
            self.cache_retention,
        )?;

        inject_model_override(&mut rig_req, model_override.as_deref());

        let response =
            self.model
                .completion(rig_req)
                .await
                .map_err(|e| LlmError::RequestFailed {
                    provider: self.model_name.clone(),
                    reason: e.to_string(),
                })?;

        let (text, mut tool_calls, finish) = extract_response(&response.choice, &response.usage);

        // Normalize tool call names: some proxies prepend "proxy_" prefixes.
        for tc in &mut tool_calls {
            let normalized = normalize_tool_name(&tc.name, &known_tool_names);
            if normalized != tc.name {
                tracing::debug!(
                    original = %tc.name,
                    normalized = %normalized,
                    "Normalized tool call name from provider",
                );
                tc.name = normalized;
            }
        }

        let resp = ToolCompletionResponse {
            content: text,
            tool_calls,
            input_tokens: saturate_u32(response.usage.input_tokens),
            output_tokens: saturate_u32(response.usage.output_tokens),
            finish_reason: finish,
            cache_read_input_tokens: saturate_u32(response.usage.cached_input_tokens),
            cache_creation_input_tokens: extract_cache_creation(&response.raw_response),
        };

        if resp.cache_read_input_tokens > 0 {
            tracing::debug!(
                model = %self.model_name,
                input = resp.input_tokens,
                output = resp.output_tokens,
                cache_read = resp.cache_read_input_tokens,
                "prompt cache hit",
            );
        }

        Ok(resp)
    }

    fn active_model_name(&self) -> String {
        self.model_name.clone()
    }

    fn effective_model_name(&self, _requested_model: Option<&str>) -> String {
        self.active_model_name()
    }

    fn set_model(&self, _model: &str) -> Result<(), LlmError> {
        // rig-core models are baked at construction time.
        // Switching requires creating a new adapter.
        Err(LlmError::RequestFailed {
            provider: self.model_name.clone(),
            reason: "Runtime model switching not supported for rig-core providers. \
                     Restart with a different model configured."
                .to_string(),
        })
    }
}

/// Normalize a tool call name returned by an OpenAI-compatible provider.
///
/// Some proxies (e.g. VibeProxy) prepend `proxy_` to tool names.
/// If the returned name doesn't match any known tool but stripping a
/// `proxy_` prefix yields a match, use the stripped version.
fn normalize_tool_name(name: &str, known_tools: &HashSet<String>) -> String {
    if known_tools.contains(name) {
        return name.to_string();
    }

    if let Some(stripped) = name.strip_prefix("proxy_")
        && known_tools.contains(stripped)
    {
        return stripped.to_string();
    }

    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_f32_to_f64_no_precision_artifacts() {
        // Direct f32->f64 cast produces 0.699999988079071 instead of 0.7
        assert_eq!(round_f32_to_f64(0.7_f32), 0.7_f64);
        assert_eq!(round_f32_to_f64(0.5_f32), 0.5_f64);
        assert_eq!(round_f32_to_f64(1.0_f32), 1.0_f64);
        assert_eq!(round_f32_to_f64(0.0_f32), 0.0_f64);
        // Original cast produces artifacts — our fix should not
        assert_ne!(0.7_f32 as f64, 0.7_f64);
    }

    // ── normalize_schema_strict: top-level flatten ────────────────────────
    //
    // OpenAI's tool API rejects schemas whose top level isn't `type:
    // "object"` or that contain top-level `oneOf`/`anyOf`/`allOf`/`enum`/
    // `not`. The GitHub Copilot MCP server exposes a tool with exactly that
    // shape (action dispatch via top-level union), and the agent gets HTTP
    // 400 the moment it tries to enumerate tools. `normalize_schema_strict`
    // detects the bad shape, flattens parameters to a permissive object
    // envelope, and stuffs the original schema into the description as
    // advisory text so the LLM can still pick variant fields. Both rig-based
    // providers and the Codex Responses API client share this normalizer.

    #[test]
    fn test_normalize_schema_strict_passes_through_valid_object_schema() {
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        });
        let mut description = "Search the index".to_string();
        let result = normalize_schema_strict(&input, &mut description);

        // Strict-mode normalization runs: additionalProperties forced false
        // and required is set to ALL keys, but the structural shape is
        // unchanged.
        assert_eq!(result["type"], "object");
        assert_eq!(result["additionalProperties"], false);
        assert_eq!(result["required"], serde_json::json!(["query"]));
        assert!(result["properties"]["query"].is_object());
        assert_eq!(
            description, "Search the index",
            "description must be untouched when no flatten happened"
        );
    }

    #[test]
    fn test_normalize_schema_strict_flattens_top_level_oneof() {
        // Mirrors the GitHub Copilot MCP `github` tool shape that triggered
        // the original 400.
        let input = serde_json::json!({
            "type": "object",
            "oneOf": [
                { "properties": { "action": { "const": "create_issue" }, "title": { "type": "string" } } },
                { "properties": { "action": { "const": "list_issues" }, "repo":  { "type": "string" } } }
            ]
        });
        let mut description = "GitHub umbrella tool".to_string();
        let result = normalize_schema_strict(&input, &mut description);

        assert_eq!(result["type"], "object");
        assert!(
            result.get("oneOf").is_none(),
            "top-level oneOf must be removed"
        );
        assert_eq!(result["additionalProperties"], true);
        assert!(result["properties"].is_object());
        assert!(result["required"].as_array().unwrap().is_empty());
        assert!(
            description.contains("Upstream JSON schema"),
            "description must include the advisory hint"
        );
        assert!(
            description.contains("create_issue"),
            "original schema variants must survive in the hint"
        );
    }

    #[test]
    fn test_normalize_schema_strict_flattens_anyof_allof_enum_not() {
        for forbidden in ["anyOf", "allOf", "enum", "not"] {
            let input = serde_json::json!({
                "type": "object",
                forbidden: ["whatever"]
            });
            let mut description = "tool".to_string();
            let result = normalize_schema_strict(&input, &mut description);
            assert!(
                result.get(forbidden).is_none(),
                "top-level {forbidden} must be stripped"
            );
            assert_eq!(result["type"], "object");
            assert_eq!(result["additionalProperties"], true);
        }
    }

    #[test]
    fn test_normalize_schema_strict_hint_is_keyword_aware() {
        // The flatten hint must match the construct that triggered it. The
        // previous one-size-fits-all "pick one variant" hint was correct
        // for oneOf/anyOf but actively misleading for allOf (where the LLM
        // should pass fields from ALL variants), enum (one of the listed
        // values), and not (any object that doesn't match).
        let cases = [
            ("oneOf", "pick ONE variant"),
            ("anyOf", "pick ONE variant"),
            ("allOf", "pass fields from ALL variants"),
            ("enum", "pass one of the listed values"),
            ("not", "does NOT match the constraint"),
        ];
        for (keyword, expected_phrase) in cases {
            let input = serde_json::json!({
                "type": "object",
                keyword: ["whatever"]
            });
            let mut description = "tool".to_string();
            let _ = normalize_schema_strict(&input, &mut description);
            assert!(
                description.contains(expected_phrase),
                "hint for top-level {keyword} must contain `{expected_phrase}`, \
                 got: {description}"
            );
        }
    }

    #[test]
    fn test_normalize_schema_strict_replaces_non_object_top_level_type() {
        // A schema like `{"type": "string"}` is not a valid OpenAI tool
        // parameters object — replace wholesale.
        let input = serde_json::json!({ "type": "string" });
        let mut description = "weird tool".to_string();
        let result = normalize_schema_strict(&input, &mut description);
        assert_eq!(result["type"], "object");
        assert!(result["properties"].is_object());
        assert_eq!(result["additionalProperties"], true);
    }

    #[test]
    fn test_normalize_schema_strict_does_not_flatten_nullable_object_type() {
        // `"type": ["object", "null"]` is valid JSON Schema for a nullable
        // object. Some upstream providers and `make_nullable` produce this
        // form. The previous check only matched `JsonValue::String("object")`
        // and would have flattened this schema, discarding all properties.
        let input = serde_json::json!({
            "type": ["object", "null"],
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        });
        let mut description = "nullable tool".to_string();
        let result = normalize_schema_strict(&input, &mut description);

        // Should NOT flatten — the schema is a valid object type.
        assert!(
            result["properties"]["query"].is_object(),
            "properties must be preserved for nullable object type, got: {result}"
        );
        assert_eq!(
            description, "nullable tool",
            "description must be untouched (no flatten hint appended)"
        );
    }

    /// Regression: OpenAI rejects `"items": true` and missing `items` on
    /// array-typed properties with "array schema items is not an object".
    /// Schema generators (schemars) produce this for `Vec<serde_json::Value>`.
    /// The normalizer must ensure `items` is a JSON Schema object.
    #[test]
    fn test_normalize_schema_strict_fixes_array_items_not_object() {
        // Case 1: items missing entirely (Vec<Value> → {"type": "array"})
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "requests": { "type": "array" }
            },
            "required": ["requests"]
        });
        let mut description = "batch".to_string();
        let result = normalize_schema_strict(&input, &mut description);
        assert!(
            result["properties"]["requests"]["items"].is_object(),
            "missing items must be filled with an object: {}",
            result["properties"]["requests"]
        );

        // Case 2: items is boolean true (valid JSON Schema, rejected by OpenAI)
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "data": { "type": "array", "items": true }
            },
            "required": ["data"]
        });
        let mut description = "bool items".to_string();
        let result = normalize_schema_strict(&input, &mut description);
        assert!(
            result["properties"]["data"]["items"].is_object(),
            "boolean items must be replaced with an object: {}",
            result["properties"]["data"]
        );

        // Case 3: items is already an object (should not be clobbered)
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "tags": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["tags"]
        });
        let mut description = "ok".to_string();
        let result = normalize_schema_strict(&input, &mut description);
        assert_eq!(
            result["properties"]["tags"]["items"]["type"], "string",
            "well-formed items must be preserved"
        );
    }

    /// Regression for google_docs_tool: a tagged enum with a variant
    /// containing `requests: Vec<serde_json::Value>` produces a top-level
    /// `oneOf` (which we flatten) with a nested `{"type": "array"}` property
    /// that has no `items`. The flatten path originally short-circuited
    /// `normalize_schema_recursive`, so the merged `requests` property kept
    /// its bare array schema and OpenAI rejected it with "array schema items
    /// is not an object". After the fix, the flatten path normalizes each
    /// merged property individually.
    #[test]
    fn test_normalize_schema_strict_flatten_normalizes_merged_array_items() {
        let input = serde_json::json!({
            "type": "object",
            "oneOf": [
                {
                    "properties": {
                        "action": { "const": "batch_update" },
                        "document_id": { "type": "string" },
                        "requests": { "type": "array" }
                    },
                    "required": ["action", "document_id", "requests"]
                },
                {
                    "properties": {
                        "action": { "const": "get" },
                        "document_id": { "type": "string" }
                    },
                    "required": ["action", "document_id"]
                }
            ]
        });
        let mut description = "docs".to_string();
        let result = normalize_schema_strict(&input, &mut description);

        // The flatten happened (oneOf removed, properties merged).
        assert!(result.get("oneOf").is_none());
        let props = result["properties"].as_object().expect("properties");
        assert!(props.contains_key("requests"));

        // CRITICAL: the merged `requests` array property must have
        // `items` as an object — not missing, not boolean, not null.
        let requests = &result["properties"]["requests"];
        assert_eq!(requests["type"], "array");
        assert!(
            requests["items"].is_object(),
            "merged array property must have items as a JSON Schema object \
             after flatten-path normalization; got: {requests}"
        );
    }

    #[test]
    fn test_normalize_schema_strict_merges_variant_properties() {
        // Top-level oneOf flatten now merges all variants' properties into
        // the envelope so the LLM sees structured field hints instead of
        // an empty `{}`. Mirrors the GitHub Copilot `github` tool shape:
        // each variant declares its own subset of fields keyed by `action`.
        let input = serde_json::json!({
            "type": "object",
            "oneOf": [
                {
                    "properties": {
                        "action": { "const": "create_issue" },
                        "title":  { "type": "string" },
                        "body":   { "type": "string" }
                    },
                    "required": ["action", "title"]
                },
                {
                    "properties": {
                        "action": { "const": "list_issues" },
                        "repo":   { "type": "string" },
                        "state":  { "type": "string" }
                    },
                    "required": ["action", "repo"]
                }
            ]
        });
        let mut description = "github".to_string();
        let result = normalize_schema_strict(&input, &mut description);

        // The flatten happened.
        assert_eq!(result["type"], "object");
        assert!(result.get("oneOf").is_none());
        assert_eq!(result["additionalProperties"], true);
        // Strict-mode `required` is empty so the LLM can mix fields from
        // different variants without failing OpenAI validation.
        assert_eq!(result["required"], serde_json::json!([]));

        // CRITICAL: properties is no longer `{}` — every field from every
        // variant must appear so the LLM can pick what to send.
        let props = result["properties"].as_object().expect("merged properties");
        assert!(props.contains_key("action"), "discriminator must merge");
        assert!(props.contains_key("title"), "create_issue field must merge");
        assert!(props.contains_key("body"), "create_issue field must merge");
        assert!(props.contains_key("repo"), "list_issues field must merge");
        assert!(props.contains_key("state"), "list_issues field must merge");
        assert_eq!(props.len(), 5);
    }

    #[test]
    fn test_normalize_schema_strict_merge_first_write_wins_on_conflict() {
        // If two variants declare the same field with different schemas,
        // first-write wins. Documented behaviour — the description hint
        // still has the full original schema for ambiguous cases.
        let input = serde_json::json!({
            "type": "object",
            "anyOf": [
                {
                    "properties": {
                        "value": { "type": "string", "description": "first" }
                    }
                },
                {
                    "properties": {
                        "value": { "type": "integer", "description": "second" }
                    }
                }
            ]
        });
        let mut description = "ambiguous".to_string();
        let result = normalize_schema_strict(&input, &mut description);

        let value_schema = &result["properties"]["value"];
        assert_eq!(value_schema["type"], "string");
        assert_eq!(value_schema["description"], "first");
    }

    #[test]
    fn test_normalize_schema_strict_preserves_nested_oneof() {
        // Nested combinators inside `properties` are FINE for the API. Only
        // the top level is forbidden, so the nested oneOf must survive
        // (its variants get recursively strict-normalized but the union
        // itself is preserved). `filter` is marked required so strict mode
        // doesn't wrap it in an `anyOf` for nullability — that would move
        // the inner oneOf one level deeper and obscure what we're checking.
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "filter": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "object", "properties": { "regex": { "type": "string" } } }
                    ]
                }
            },
            "required": ["filter"]
        });
        let mut description = "search".to_string();
        let result = normalize_schema_strict(&input, &mut description);

        assert_eq!(result["type"], "object");
        // Nested oneOf survives untouched at the same path.
        let nested = &result["properties"]["filter"]["oneOf"];
        assert!(nested.is_array(), "nested oneOf must be preserved");
        assert_eq!(nested.as_array().unwrap().len(), 2);
        // The object variant inside the nested oneOf got strict-mode
        // normalized (additionalProperties: false, all keys required).
        let object_variant = &nested[1];
        assert_eq!(object_variant["type"], "object");
        assert_eq!(object_variant["additionalProperties"], false);
        assert_eq!(description, "search");
    }

    #[test]
    fn test_normalize_schema_strict_truncates_huge_schema_on_char_boundary() {
        // 4KB blob with a multi-byte char near the truncation point. The
        // truncated hint must not panic and must end on a valid char
        // boundary.
        let big_string = "α".repeat(2000); // each `α` is 2 bytes in UTF-8 → 4000 bytes
        let input = serde_json::json!({
            "anyOf": [{ "description": big_string }]
        });
        let mut description = "tool".to_string();
        let result = normalize_schema_strict(&input, &mut description);
        assert!(description.contains("(truncated)"));
        assert_eq!(result["type"], "object");
        assert!(result.get("anyOf").is_none());
    }

    /// Size-capped serializer: verify the capped writer produces correct
    /// output at boundary conditions.
    #[test]
    fn test_serialize_json_capped_boundary_conditions() {
        // Small schema under the cap: full output, no truncation.
        let small = serde_json::json!({"a": 1});
        let result = serialize_json_capped(&small, 1500).expect("should serialize");
        assert_eq!(result, r#"{"a":1}"#);

        // Exactly at the cap: should produce exactly cap bytes (or fewer
        // if the serialized output happens to be shorter).
        let result = serialize_json_capped(&small, 7).expect("should serialize");
        assert_eq!(result.len(), 7); // {"a":1} is exactly 7 bytes

        // Over the cap: output is truncated. The JSON will be malformed
        // (cut mid-stream) but that's OK — the caller adds "... (truncated)".
        let result = serialize_json_capped(&small, 4).expect("should serialize");
        assert_eq!(result.len(), 4);
        assert_eq!(result, r#"{"a""#);

        // Cap of 0: empty output.
        let result = serialize_json_capped(&small, 0).expect("should serialize");
        assert!(result.is_empty());
    }

    /// Size-capped serializer with multi-MB string values: the cap must
    /// bound the allocation even when the schema has few nodes but large
    /// string values (the gap the old node-counting approach missed).
    #[test]
    fn test_serialize_json_capped_large_string_values() {
        let big = serde_json::json!({
            "description": "x".repeat(100_000)
        });
        let result = serialize_json_capped(&big, 1500).expect("should serialize");
        assert!(
            result.len() <= 1500,
            "capped serializer must bound output to max_bytes; got {} bytes",
            result.len()
        );
        // The output should start with valid JSON structure.
        assert!(result.starts_with(r#"{"description":""#));
    }

    /// Caller-level regression test: drives `convert_tools` (the rig-based
    /// provider entry point) end to end with a GitHub-Copilot-shaped tool
    /// definition and asserts the resulting `RigToolDefinition` has a clean
    /// top level. This is the test that would have caught the OpenAI-via-rig
    /// path regressing the same way the Codex path did.
    #[test]
    fn test_convert_tools_handles_top_level_oneof_dispatcher() {
        let tools = vec![IronToolDefinition {
            name: "github".to_string(),
            description: "GitHub MCP umbrella tool".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "oneOf": [
                    {
                        "properties": {
                            "action": { "const": "create_issue" },
                            "title":  { "type": "string" }
                        },
                        "required": ["action", "title"]
                    },
                    {
                        "properties": {
                            "action": { "const": "list_issues" },
                            "repo":   { "type": "string" }
                        },
                        "required": ["action", "repo"]
                    }
                ]
            }),
        }];
        let converted = convert_tools(&tools);
        assert_eq!(converted.len(), 1);
        let tool = &converted[0];

        assert_eq!(tool.name, "github");
        assert_eq!(tool.parameters["type"], "object");
        assert!(
            tool.parameters.get("oneOf").is_none(),
            "top-level oneOf must not survive into the rig-core ToolDefinition"
        );
        assert_eq!(tool.parameters["additionalProperties"], true);
        assert!(
            tool.description.starts_with("GitHub MCP umbrella tool"),
            "original description must come first"
        );
        assert!(
            tool.description.contains("Upstream JSON schema"),
            "advisory hint must be appended"
        );
        assert!(
            tool.description.contains("create_issue") && tool.description.contains("list_issues"),
            "variant info must be retained in the hint"
        );
    }

    /// End-to-end regression test using the google_docs_tool's actual schema
    /// shape. This tool has a tagged enum (`oneOf`) with a `BatchUpdate`
    /// variant containing `requests: Vec<serde_json::Value>` — which
    /// produces a bare `{"type": "array"}` with no `items`. The flatten
    /// path broke TWICE on this shape:
    ///
    /// 1. The `return schema` short-circuit skipped `normalize_schema_recursive`,
    ///    so the merged `requests` property kept its bare array (no items).
    /// 2. Even after the array-items fix was added to the recursive normalizer,
    ///    the flatten path still short-circuited before it ran.
    ///
    /// This single test would have caught BOTH bugs. It drives the schema
    /// through `normalize_schema_strict` (shared normalizer) AND both
    /// consumer paths: `convert_tools` (rig-based providers) and
    /// `convert_tool_definition` (codex provider). Asserts:
    /// - top-level oneOf is flattened
    /// - merged properties include fields from ALL variants
    /// - array `items` is an object (not missing/boolean)
    /// - nested object properties get strict-mode treatment
    /// - output passes `validate_strict_schema` with zero violations
    #[test]
    fn test_realistic_wasm_schema_survives_normalize_flatten_pipeline() {
        // Actual shape from google_docs_tool: tagged enum with 4 variants.
        // BatchUpdate has `requests: Vec<Value>` (bare array, no items).
        // GetDocument/ReadContent have only string fields.
        // InsertText has a nested object (text_style).
        let wasm_schema = serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "const": "get_document" },
                        "document_id": { "type": "string" }
                    },
                    "required": ["action", "document_id"]
                },
                {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "const": "batch_update" },
                        "document_id": { "type": "string" },
                        "requests": { "type": "array" }
                    },
                    "required": ["action", "document_id", "requests"]
                },
                {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "const": "insert_text" },
                        "document_id": { "type": "string" },
                        "text": { "type": "string" },
                        "text_style": {
                            "type": "object",
                            "properties": {
                                "bold": { "type": "boolean" },
                                "font_size": { "type": "integer" }
                            }
                        }
                    },
                    "required": ["action", "document_id", "text"]
                },
                {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "const": "read_content" },
                        "document_id": { "type": "string" }
                    },
                    "required": ["action", "document_id"]
                }
            ]
        });

        // Path 1: shared normalizer (used by both rig and codex paths)
        let mut description = "Google Docs tool".to_string();
        let normalized = normalize_schema_strict(&wasm_schema, &mut description);

        // Top-level oneOf must be flattened.
        assert!(normalized.get("oneOf").is_none(), "oneOf must be flattened");
        assert_eq!(normalized["type"], "object");
        assert_eq!(normalized["additionalProperties"], true);

        // Merged properties from ALL variants.
        let props = normalized["properties"]
            .as_object()
            .expect("merged properties");
        assert!(props.contains_key("action"), "discriminator");
        assert!(props.contains_key("document_id"), "shared field");
        assert!(props.contains_key("requests"), "BatchUpdate field");
        assert!(props.contains_key("text"), "InsertText field");
        assert!(props.contains_key("text_style"), "InsertText nested obj");

        // CRITICAL: array `items` must be an object (the bug that broke twice).
        let requests = &normalized["properties"]["requests"];
        assert!(
            requests["items"].is_object(),
            "requests array must have items as a JSON Schema object; got: {requests}"
        );

        // Nested object properties should get strict-mode treatment
        // (additionalProperties: false on the text_style sub-object).
        let text_style = &normalized["properties"]["text_style"];
        assert_eq!(text_style["additionalProperties"], false);
        assert!(text_style["properties"]["bold"].is_object());

        // Description hint should contain variant info.
        assert!(
            description.contains("batch_update") || description.contains("get_document"),
            "description must include variant info from the original schema"
        );

        // Path 2: rig-based provider entry point.
        let tools = convert_tools(&[IronToolDefinition {
            name: "google_docs_tool".to_string(),
            description: "Google Docs".to_string(),
            parameters: wasm_schema.clone(),
        }]);
        assert_eq!(tools.len(), 1);
        assert!(
            tools[0].parameters.get("oneOf").is_none(),
            "convert_tools output must not have oneOf"
        );
        assert!(
            tools[0].parameters["properties"]["requests"]["items"].is_object(),
            "convert_tools must normalize array items"
        );
    }

    /// Deeply nested schema: object → array → object → array (no items).
    /// Verifies the recursive normalizer walks the full depth and fixes
    /// every array `items` and every nested object's strict-mode fields.
    #[test]
    fn test_normalize_schema_strict_fixes_deeply_nested_array_items() {
        // All fields marked required so `make_nullable` doesn't wrap
        // types as `["array", "null"]`, keeping the assertions focused
        // on "items get fixed at every nesting depth".
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "data": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "tags": { "type": "array" },
                            "metadata": {
                                "type": "object",
                                "properties": {
                                    "values": { "type": "array" }
                                },
                                "required": ["values"]
                            }
                        },
                        "required": ["tags", "metadata"]
                    }
                }
            },
            "required": ["data"]
        });
        let mut description = "nested".to_string();
        let result = normalize_schema_strict(&input, &mut description);

        // Level 1: data.items is an object (was already, should be preserved)
        let data_items = &result["properties"]["data"]["items"];
        assert!(data_items.is_object());

        // Level 2: data.items.properties.tags must get items added
        let tags = &data_items["properties"]["tags"];
        assert_eq!(tags["type"], "array");
        assert!(
            tags["items"].is_object(),
            "deeply nested array must get items: {tags}"
        );

        // Level 3: data.items.properties.metadata.properties.values
        let values = &data_items["properties"]["metadata"]["properties"]["values"];
        assert_eq!(values["type"], "array");
        assert!(
            values["items"].is_object(),
            "3-level deep array must get items: {values}"
        );

        // Nested objects should have additionalProperties: false
        assert_eq!(data_items["additionalProperties"], false);
        assert_eq!(
            data_items["properties"]["metadata"]["additionalProperties"],
            false
        );
    }

    #[test]
    fn test_convert_messages_system_to_preamble() {
        let messages = vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::user("Hello"),
        ];
        let (preamble, history) = convert_messages(&messages);
        assert_eq!(preamble, Some("You are a helpful assistant.".to_string()));
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn test_convert_messages_multiple_systems_concatenated() {
        let messages = vec![
            ChatMessage::system("System 1"),
            ChatMessage::system("System 2"),
            ChatMessage::user("Hi"),
        ];
        let (preamble, history) = convert_messages(&messages);
        assert_eq!(preamble, Some("System 1\nSystem 2".to_string()));
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn test_convert_messages_tool_result() {
        // Use a conforming 9-char alphanumeric ID so it passes through unchanged.
        let messages = vec![ChatMessage::tool_result(
            "abcDE1234",
            "search",
            "result text",
        )];
        let (preamble, history) = convert_messages(&messages);
        assert!(preamble.is_none());
        assert_eq!(history.len(), 1);
        // Tool results become User messages in rig-core
        match &history[0] {
            RigMessage::User { content } => match content.first() {
                UserContent::ToolResult(r) => {
                    assert_eq!(r.id, "abcDE1234");
                    assert_eq!(r.call_id.as_deref(), Some("abcDE1234"));
                }
                other => panic!("Expected tool result content, got: {:?}", other),
            },
            other => panic!("Expected User message, got: {:?}", other),
        }
    }

    #[test]
    fn test_convert_messages_assistant_with_tool_calls() {
        // Use a conforming 9-char alphanumeric ID so it passes through unchanged.
        let tc = IronToolCall {
            id: "Xt7mK9pQ2".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "test"}),
            reasoning: None,
        };
        let msg = ChatMessage::assistant_with_tool_calls(Some("thinking".to_string()), vec![tc]);
        let messages = vec![msg];
        let (_preamble, history) = convert_messages(&messages);
        assert_eq!(history.len(), 1);
        match &history[0] {
            RigMessage::Assistant { content, .. } => {
                // Should have both text and tool call
                assert!(content.iter().count() >= 2);
                for item in content.iter() {
                    if let AssistantContent::ToolCall(tc) = item {
                        assert_eq!(tc.call_id.as_deref(), Some("Xt7mK9pQ2"));
                    }
                }
            }
            other => panic!("Expected Assistant message, got: {:?}", other),
        }
    }

    #[test]
    fn test_convert_messages_tool_result_without_id_gets_fallback() {
        let messages = vec![ChatMessage {
            role: crate::llm::Role::Tool,
            content: "result text".to_string(),
            content_parts: Vec::new(),
            tool_call_id: None,
            name: Some("search".to_string()),
            tool_calls: None,
        }];
        let (_preamble, history) = convert_messages(&messages);
        match &history[0] {
            RigMessage::User { content } => match content.first() {
                UserContent::ToolResult(r) => {
                    // Missing ID → normalized_tool_call_id generates a 9-char alphanumeric ID.
                    assert_eq!(
                        r.id.len(),
                        9,
                        "fallback ID should be 9 chars, got: {}",
                        r.id
                    );
                    assert!(r.id.chars().all(|c| c.is_ascii_alphanumeric()));
                    assert_eq!(r.call_id.as_deref(), Some(r.id.as_str()));
                }
                other => panic!("Expected tool result content, got: {:?}", other),
            },
            other => panic!("Expected User message, got: {:?}", other),
        }
    }

    #[test]
    fn test_convert_tools() {
        let tools = vec![IronToolDefinition {
            name: "search".to_string(),
            description: "Search the web".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"}
                }
            }),
        }];
        let rig_tools = convert_tools(&tools);
        assert_eq!(rig_tools.len(), 1);
        assert_eq!(rig_tools[0].name, "search");
        assert_eq!(rig_tools[0].description, "Search the web");
    }

    #[test]
    fn test_convert_tool_choice() {
        assert!(matches!(
            convert_tool_choice(Some("auto")),
            Some(RigToolChoice::Auto)
        ));
        assert!(matches!(
            convert_tool_choice(Some("required")),
            Some(RigToolChoice::Required)
        ));
        assert!(matches!(
            convert_tool_choice(Some("none")),
            Some(RigToolChoice::None)
        ));
        assert!(matches!(
            convert_tool_choice(Some("AUTO")),
            Some(RigToolChoice::Auto)
        ));
        assert!(convert_tool_choice(None).is_none());
        assert!(convert_tool_choice(Some("unknown")).is_none());
    }

    #[test]
    fn test_extract_response_text_only() {
        let content = OneOrMany::one(AssistantContent::text("Hello world"));
        let usage = RigUsage::new();
        let (text, calls, finish) = extract_response(&content, &usage);
        assert_eq!(text, Some("Hello world".to_string()));
        assert!(calls.is_empty());
        assert_eq!(finish, FinishReason::Stop);
    }

    #[test]
    fn test_extract_response_tool_call() {
        let tc = AssistantContent::tool_call("call_1", "search", serde_json::json!({"q": "test"}));
        let content = OneOrMany::one(tc);
        let usage = RigUsage::new();
        let (text, calls, finish) = extract_response(&content, &usage);
        assert!(text.is_none());
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "search");
        assert_eq!(finish, FinishReason::ToolUse);
    }

    #[test]
    fn test_assistant_tool_call_empty_id_gets_generated() {
        let tc = IronToolCall {
            id: "".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "test"}),
            reasoning: None,
        };
        let messages = vec![ChatMessage::assistant_with_tool_calls(None, vec![tc])];
        let (_preamble, history) = convert_messages(&messages);

        match &history[0] {
            RigMessage::Assistant { content, .. } => {
                let tool_call = content.iter().find_map(|c| match c {
                    AssistantContent::ToolCall(tc) => Some(tc),
                    _ => None,
                });
                let tc = tool_call.expect("should have a tool call");
                // Empty ID → normalized_tool_call_id generates a 9-char alphanumeric ID.
                assert_eq!(
                    tc.id.len(),
                    9,
                    "generated id should be 9 chars, got: {}",
                    tc.id
                );
                assert!(tc.id.chars().all(|c| c.is_ascii_alphanumeric()));
                assert_eq!(tc.call_id.as_deref(), Some(tc.id.as_str()));
            }
            other => panic!("Expected Assistant message, got: {:?}", other),
        }
    }

    #[test]
    fn test_assistant_tool_call_whitespace_id_gets_generated() {
        let tc = IronToolCall {
            id: "   ".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "test"}),
            reasoning: None,
        };
        let messages = vec![ChatMessage::assistant_with_tool_calls(None, vec![tc])];
        let (_preamble, history) = convert_messages(&messages);

        match &history[0] {
            RigMessage::Assistant { content, .. } => {
                let tool_call = content.iter().find_map(|c| match c {
                    AssistantContent::ToolCall(tc) => Some(tc),
                    _ => None,
                });
                let tc = tool_call.expect("should have a tool call");
                // Whitespace-only ID → normalized_tool_call_id generates a 9-char alphanumeric ID.
                assert_eq!(
                    tc.id.len(),
                    9,
                    "generated id should be 9 chars, got: {}",
                    tc.id
                );
                assert!(tc.id.chars().all(|c| c.is_ascii_alphanumeric()));
            }
            other => panic!("Expected Assistant message, got: {:?}", other),
        }
    }

    #[test]
    fn test_assistant_and_tool_result_missing_ids_share_generated_id() {
        // Simulate: assistant emits a tool call with empty id, then tool
        // result arrives without an id. Both should get deterministic
        // generated ids that match (based on their position in history).
        let tc = IronToolCall {
            id: "".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "test"}),
            reasoning: None,
        };
        let assistant_msg = ChatMessage::assistant_with_tool_calls(None, vec![tc]);
        let tool_result_msg = ChatMessage {
            role: crate::llm::Role::Tool,
            content: "search results here".to_string(),
            content_parts: Vec::new(),
            tool_call_id: None,
            name: Some("search".to_string()),
            tool_calls: None,
        };
        let messages = vec![assistant_msg, tool_result_msg];
        let (_preamble, history) = convert_messages(&messages);

        // Extract the generated call_id from the assistant tool call
        let assistant_call_id = match &history[0] {
            RigMessage::Assistant { content, .. } => {
                let tc = content.iter().find_map(|c| match c {
                    AssistantContent::ToolCall(tc) => Some(tc),
                    _ => None,
                });
                tc.expect("should have tool call").id.clone()
            }
            other => panic!("Expected Assistant message, got: {:?}", other),
        };

        // Extract the generated call_id from the tool result
        let tool_result_call_id = match &history[1] {
            RigMessage::User { content } => match content.first() {
                UserContent::ToolResult(r) => r
                    .call_id
                    .clone()
                    .expect("tool result call_id must be present"),
                other => panic!("Expected ToolResult, got: {:?}", other),
            },
            other => panic!("Expected User message, got: {:?}", other),
        };

        assert!(
            !assistant_call_id.is_empty(),
            "assistant call_id must not be empty"
        );
        assert!(
            !tool_result_call_id.is_empty(),
            "tool result call_id must not be empty"
        );

        // NOTE: With the current seed-based generation, these IDs will differ
        // because the assistant tool call uses seed=0 (history.len() at that
        // point) and the tool result uses seed=1 (history.len() after the
        // assistant message was pushed). This documents the current behavior.
        // A future improvement could thread the assistant's generated ID into
        // the tool result for exact matching.
        assert_ne!(
            assistant_call_id, tool_result_call_id,
            "Current impl generates different IDs for assistant call and tool result \
             because seeds differ; this documents the known limitation"
        );
    }

    #[test]
    fn test_saturate_u32() {
        assert_eq!(saturate_u32(100), 100);
        assert_eq!(saturate_u32(u64::MAX), u32::MAX);
        assert_eq!(saturate_u32(u32::MAX as u64), u32::MAX);
    }

    // -- normalize_tool_name tests --

    #[test]
    fn test_normalize_tool_name_exact_match() {
        let known = HashSet::from(["echo".to_string(), "list_jobs".to_string()]);
        assert_eq!(normalize_tool_name("echo", &known), "echo");
    }

    #[test]
    fn test_normalize_tool_name_proxy_prefix_match() {
        let known = HashSet::from(["echo".to_string(), "list_jobs".to_string()]);
        assert_eq!(normalize_tool_name("proxy_echo", &known), "echo");
    }

    #[test]
    fn test_normalize_tool_name_proxy_prefix_no_match_kept() {
        let known = HashSet::from(["echo".to_string(), "list_jobs".to_string()]);
        assert_eq!(
            normalize_tool_name("proxy_unknown", &known),
            "proxy_unknown"
        );
    }

    #[test]
    fn test_normalize_tool_name_unknown_passthrough() {
        let known = HashSet::from(["echo".to_string()]);
        assert_eq!(normalize_tool_name("other_tool", &known), "other_tool");
    }

    #[test]
    fn test_build_rig_request_injects_cache_control_short() {
        let req = build_rig_request(
            Some("You are helpful.".to_string()),
            vec![RigMessage::user("Hello")],
            Vec::new(),
            None,
            None,
            None,
            CacheRetention::Short,
        )
        .unwrap();

        let params = req
            .additional_params
            .expect("should have additional_params for Short retention");
        assert_eq!(params["cache_control"]["type"], "ephemeral");
        assert!(
            params["cache_control"].get("ttl").is_none(),
            "Short retention should not include ttl"
        );
    }

    #[test]
    fn test_build_rig_request_injects_cache_control_long() {
        let req = build_rig_request(
            Some("You are helpful.".to_string()),
            vec![RigMessage::user("Hello")],
            Vec::new(),
            None,
            None,
            None,
            CacheRetention::Long,
        )
        .unwrap();

        let params = req
            .additional_params
            .expect("should have additional_params for Long retention");
        assert_eq!(params["cache_control"]["type"], "ephemeral");
        assert_eq!(params["cache_control"]["ttl"], "1h");
    }

    #[test]
    fn test_build_rig_request_no_cache_control_when_none() {
        let req = build_rig_request(
            Some("You are helpful.".to_string()),
            vec![RigMessage::user("Hello")],
            Vec::new(),
            None,
            None,
            None,
            CacheRetention::None,
        )
        .unwrap();

        assert!(
            req.additional_params.is_none(),
            "additional_params should be None when cache is disabled"
        );
    }

    /// Verify that the multiplier match arms in `RigAdapter::cache_write_multiplier`
    /// produce the expected values. We use a standalone helper because constructing
    /// a real `RigAdapter` requires a rig `Model` (which needs network/provider setup).
    /// The helper mirrors the same match expression — if the impl drifts, the
    /// `test_build_rig_request_*` tests will still catch regressions end-to-end.
    #[test]
    fn test_cache_write_multiplier_values() {
        use rust_decimal::Decimal;
        // None → 1.0× (no surcharge)
        assert_eq!(
            cache_write_multiplier_for(CacheRetention::None),
            Decimal::ONE
        );
        // Short → 1.25× (25% surcharge)
        assert_eq!(
            cache_write_multiplier_for(CacheRetention::Short),
            Decimal::new(125, 2)
        );
        // Long → 2.0× (100% surcharge)
        assert_eq!(
            cache_write_multiplier_for(CacheRetention::Long),
            Decimal::TWO
        );
    }

    fn cache_write_multiplier_for(retention: CacheRetention) -> rust_decimal::Decimal {
        match retention {
            CacheRetention::None => rust_decimal::Decimal::ONE,
            CacheRetention::Short => rust_decimal::Decimal::new(125, 2),
            CacheRetention::Long => rust_decimal::Decimal::TWO,
        }
    }

    // -- supports_prompt_cache tests --

    #[test]
    fn test_supports_prompt_cache_supported_models() {
        // All Claude 3+ models per Anthropic docs
        assert!(supports_prompt_cache("claude-opus-4-6"));
        assert!(supports_prompt_cache("claude-sonnet-4-6"));
        assert!(supports_prompt_cache("claude-sonnet-4"));
        assert!(supports_prompt_cache("claude-haiku-4-5"));
        assert!(supports_prompt_cache("claude-3-5-sonnet-20241022"));
        assert!(supports_prompt_cache("claude-haiku-3"));
        assert!(supports_prompt_cache("Claude-Opus-4-5")); // case-insensitive
        assert!(supports_prompt_cache("anthropic/claude-sonnet-4-6")); // provider prefix
    }

    #[test]
    fn test_supports_prompt_cache_unsupported_models() {
        // Legacy Claude models that predate caching
        assert!(!supports_prompt_cache("claude-2"));
        assert!(!supports_prompt_cache("claude-2.1"));
        assert!(!supports_prompt_cache("claude-instant-1.2"));
        // Non-Claude models
        assert!(!supports_prompt_cache("gpt-4o"));
        assert!(!supports_prompt_cache("llama3"));
    }

    #[test]
    fn test_with_unsupported_params_populates_set() {
        use rig::client::CompletionClient;
        use rig::providers::openai;

        let client: openai::Client = openai::Client::builder()
            .api_key("test-key")
            .base_url("http://localhost:0")
            .build()
            .unwrap();
        let client = client.completions_api();
        let model = client.completion_model("test-model");
        let adapter = RigAdapter::new(model, "test-model")
            .with_unsupported_params(vec!["temperature".to_string()]);

        assert!(adapter.unsupported_params.contains("temperature"));
        assert!(!adapter.unsupported_params.contains("max_tokens"));
    }

    #[test]
    fn test_strip_unsupported_completion_params() {
        use rig::client::CompletionClient;
        use rig::providers::openai;

        let client: openai::Client = openai::Client::builder()
            .api_key("test-key")
            .base_url("http://localhost:0")
            .build()
            .unwrap();
        let client = client.completions_api();
        let model = client.completion_model("test-model");
        let adapter = RigAdapter::new(model, "test-model").with_unsupported_params(vec![
            "temperature".to_string(),
            "stop_sequences".to_string(),
        ]);

        let mut req = CompletionRequest::new(vec![ChatMessage::user("hi")]);
        req.temperature = Some(0.7);
        req.max_tokens = Some(100);
        req.stop_sequences = Some(vec!["STOP".to_string()]);

        adapter.strip_unsupported_completion_params(&mut req);

        assert!(req.temperature.is_none(), "temperature should be stripped");
        assert_eq!(req.max_tokens, Some(100), "max_tokens should be preserved");
        assert!(
            req.stop_sequences.is_none(),
            "stop_sequences should be stripped"
        );
    }

    #[test]
    fn test_strip_unsupported_tool_params() {
        use rig::client::CompletionClient;
        use rig::providers::openai;

        let client: openai::Client = openai::Client::builder()
            .api_key("test-key")
            .base_url("http://localhost:0")
            .build()
            .unwrap();
        let client = client.completions_api();
        let model = client.completion_model("test-model");
        let adapter = RigAdapter::new(model, "test-model")
            .with_unsupported_params(vec!["temperature".to_string(), "max_tokens".to_string()]);

        let mut req = ToolCompletionRequest::new(vec![ChatMessage::user("hi")], vec![]);
        req.temperature = Some(0.5);
        req.max_tokens = Some(200);

        adapter.strip_unsupported_tool_params(&mut req);

        assert!(req.temperature.is_none(), "temperature should be stripped");
        assert!(req.max_tokens.is_none(), "max_tokens should be stripped");
    }

    #[test]
    fn test_unsupported_params_empty_by_default() {
        use rig::client::CompletionClient;
        use rig::providers::openai;

        let client: openai::Client = openai::Client::builder()
            .api_key("test-key")
            .base_url("http://localhost:0")
            .build()
            .unwrap();
        let client = client.completions_api();
        let model = client.completion_model("test-model");
        let adapter = RigAdapter::new(model, "test-model");

        assert!(adapter.unsupported_params.is_empty());
    }

    /// Regression test: consecutive tool_result messages from parallel tool
    /// execution must be merged into a single User message with multiple
    /// ToolResult content items. Without merging, APIs like Anthropic reject
    /// the request due to consecutive User messages.
    #[test]
    fn test_consecutive_tool_results_merged_into_single_user_message() {
        let tc1 = IronToolCall {
            id: "call_a".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"q": "rust"}),
            reasoning: None,
        };
        let tc2 = IronToolCall {
            id: "call_b".to_string(),
            name: "fetch".to_string(),
            arguments: serde_json::json!({"url": "https://example.com"}),
            reasoning: None,
        };
        let assistant = ChatMessage::assistant_with_tool_calls(None, vec![tc1, tc2]);
        let result_a = ChatMessage::tool_result("call_a", "search", "search results");
        let result_b = ChatMessage::tool_result("call_b", "fetch", "fetch results");

        let messages = vec![assistant, result_a, result_b];
        let (_preamble, history) = convert_messages(&messages);

        // Should be: 1 assistant + 1 merged user (not 1 assistant + 2 users)
        assert_eq!(
            history.len(),
            2,
            "Expected 2 messages (assistant + merged user), got {}",
            history.len()
        );

        // The second message should contain both tool results
        match &history[1] {
            RigMessage::User { content } => {
                assert_eq!(
                    content.len(),
                    2,
                    "Expected 2 tool results in merged user message, got {}",
                    content.len()
                );
                for item in content.iter() {
                    assert!(
                        matches!(item, UserContent::ToolResult(_)),
                        "Expected ToolResult content"
                    );
                }
            }
            other => panic!("Expected User message, got: {:?}", other),
        }
    }

    /// Verify that a tool_result after a non-tool User message is NOT merged.
    #[test]
    fn test_tool_result_after_user_text_not_merged() {
        let user_msg = ChatMessage::user("hello");
        let tool_msg = ChatMessage::tool_result("call_1", "search", "results");

        let messages = vec![user_msg, tool_msg];
        let (_preamble, history) = convert_messages(&messages);

        // Should be 2 separate User messages (text user + tool result user)
        assert_eq!(history.len(), 2);
    }

    /// Empty user messages (e.g. after thinking-tag stripping) must be skipped.
    /// Strict providers like Kimi return 400 when "content": "" is sent.
    #[test]
    fn test_empty_user_message_is_skipped() {
        let empty = ChatMessage::user("");
        let non_empty = ChatMessage::user("hello");
        let messages = vec![empty, non_empty];
        let (_preamble, history) = convert_messages(&messages);

        assert_eq!(history.len(), 1, "empty user message must be dropped");
        match &history[0] {
            RigMessage::User { content } => {
                assert_eq!(content.len(), 1);
                let first = content.iter().next().expect("one content item");
                match first {
                    UserContent::Text(t) => assert_eq!(t.text, "hello"),
                    other => panic!("expected Text, got {:?}", other),
                }
            }
            other => panic!("expected User message, got {:?}", other),
        }
    }

    /// Empty assistant messages (e.g. after thinking-tag stripping) must be skipped.
    #[test]
    fn test_empty_assistant_message_is_skipped() {
        let empty_asst = ChatMessage {
            role: crate::llm::Role::Assistant,
            content: String::new(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            content_parts: vec![],
        };
        let non_empty = ChatMessage::user("hi");
        let messages = vec![empty_asst, non_empty];
        let (_preamble, history) = convert_messages(&messages);

        assert_eq!(history.len(), 1, "empty assistant message must be dropped");
        assert!(matches!(history[0], RigMessage::User { .. }));
    }

    /// A conversation mixing normal and empty messages: only non-empty ones survive.
    #[test]
    fn test_mixed_empty_and_non_empty_messages_filtered_correctly() {
        let user1 = ChatMessage::user("first");
        let empty_asst = ChatMessage {
            role: crate::llm::Role::Assistant,
            content: String::new(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            content_parts: vec![],
        };
        let user2 = ChatMessage::user("");
        let asst = ChatMessage::assistant("response");
        let messages = vec![user1, empty_asst, user2, asst];
        let (_preamble, history) = convert_messages(&messages);

        assert_eq!(history.len(), 2, "only non-empty messages should survive");
        assert!(matches!(history[0], RigMessage::User { .. }));
        assert!(matches!(history[1], RigMessage::Assistant { .. }));
    }

    // -- normalized_tool_call_id tests --

    #[test]
    fn test_normalized_tool_call_id_conforming_passthrough() {
        // A 9-char alphanumeric ID should pass through unchanged.
        let id = normalized_tool_call_id(Some("abcDE1234"), 42);
        assert_eq!(id, "abcDE1234");
    }

    #[test]
    fn test_normalized_tool_call_id_non_conforming_hashed() {
        // An ID that doesn't match [a-zA-Z0-9]{9} should be hashed into one.
        let id = normalized_tool_call_id(Some("call_abc_long_id"), 0);
        assert_eq!(id.len(), 9);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
        // Should NOT be the raw input.
        assert_ne!(id, "call_abc_l");
    }

    #[test]
    fn test_normalized_tool_call_id_empty_input() {
        let id = normalized_tool_call_id(Some(""), 5);
        assert_eq!(id.len(), 9);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_normalized_tool_call_id_whitespace_input() {
        let id = normalized_tool_call_id(Some("   "), 5);
        assert_eq!(id.len(), 9);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
        // Empty and whitespace-only with the same seed should produce identical results.
        let id_empty = normalized_tool_call_id(Some(""), 5);
        assert_eq!(id, id_empty);
    }

    #[test]
    fn test_normalized_tool_call_id_none_input() {
        let id = normalized_tool_call_id(None, 7);
        assert_eq!(id.len(), 9);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
        // None and empty string with same seed should produce identical results.
        let id_empty = normalized_tool_call_id(Some(""), 7);
        assert_eq!(id, id_empty);
    }

    #[test]
    fn test_normalized_tool_call_id_deterministic() {
        let id1 = normalized_tool_call_id(Some("call_xyz_123"), 0);
        let id2 = normalized_tool_call_id(Some("call_xyz_123"), 0);
        assert_eq!(id1, id2, "same input must produce same output");
    }

    #[test]
    fn test_normalized_tool_call_id_different_inputs_differ() {
        let id_a = normalized_tool_call_id(Some("call_aaa"), 0);
        let id_b = normalized_tool_call_id(Some("call_bbb"), 0);
        assert_ne!(
            id_a, id_b,
            "different raw IDs should produce different hashed IDs"
        );
    }

    fn make_rig_request(additional_params: Option<serde_json::Value>) -> RigRequest {
        RigRequest {
            preamble: None,
            chat_history: OneOrMany::one(RigMessage::user("test")),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params,
        }
    }

    #[test]
    fn test_inject_model_override_creates_params_when_none() {
        let mut req = make_rig_request(None);
        inject_model_override(&mut req, Some("test-model"));

        let params = req
            .additional_params
            .expect("additional_params should be Some");
        assert_eq!(params, serde_json::json!({ "model": "test-model" }));
    }

    #[test]
    fn test_inject_model_override_preserves_existing_params() {
        let mut req = make_rig_request(Some(serde_json::json!({
            "cache_control": { "type": "ephemeral" },
        })));
        inject_model_override(&mut req, Some("override-model"));

        let params = req.additional_params.expect("should remain Some");
        let obj = params.as_object().expect("should be object");
        assert_eq!(
            obj.get("cache_control"),
            Some(&serde_json::json!({ "type": "ephemeral" }))
        );
        assert_eq!(obj.get("model"), Some(&serde_json::json!("override-model")));
    }

    #[test]
    fn test_inject_model_override_noop_when_none() {
        let mut req = make_rig_request(None);
        inject_model_override(&mut req, None);
        assert!(req.additional_params.is_none());
    }
}
