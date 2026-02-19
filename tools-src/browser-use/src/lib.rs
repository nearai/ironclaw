wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

mod cdp;
mod cdp_actions;
mod constants;
mod dispatch;
mod envelope;
mod error;
mod normalize;
mod session;
mod validation;

use serde_json::{json, Map, Value};

use crate::constants::*;
use crate::dispatch::dispatch_with_retries;
use crate::envelope::{error_envelope, success_envelope};
use crate::error::StructuredError;
use crate::normalize::{alias_note, normalize_action};
use crate::validation::{
    extract_optional_session_id, resolve_backend_url, resolve_timeout_ms, validate_action_params,
};

struct BrowserUseTool;

impl exports::near::agent::tool::Guest for BrowserUseTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        let output = execute_inner(&req.params);
        exports::near::agent::tool::Response {
            output: Some(output),
            error: None,
        }
    }

    fn schema() -> String {
        json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": CANONICAL_ACTIONS,
                    "description": "Canonical browser-use action (aliases like goto/navigate normalize to open)."
                },
                "url": {
                    "type": "string",
                    "description": "Navigation URL for action=open. Must start with http:// or https://."
                },
                "session_id": {
                    "type": "string",
                    "description": "Browser session identifier. Required for stateful actions except session_create/session_list."
                },
                "ref": {
                    "type": "string",
                    "description": "Deterministic snapshot ref, format @eN (e.g. @e12)."
                },
                "selector": {
                    "type": "string",
                    "description": "Semantic selector for DOM actions (click/fill/etc). Not used as navigation URL for open."
                },
                "timeout_ms": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_ACTION_TIMEOUT_MS,
                    "description": "Optional per-action timeout override in milliseconds."
                },
                "backend_url": {
                    "type": "string",
                    "description": "Optional Browserless sidecar endpoint override. Must target localhost."
                }
            },
            "additionalProperties": true
        })
        .to_string()
    }

    fn description() -> String {
        "Browser-use tool for IronClaw using CDP WebSocket protocol for persistent browser sessions. \
         Provides feature parity with agent-browser: navigation (open/back/forward/reload), DOM snapshots, \
         element interactions (click/type/fill/hover/select/scroll), content retrieval (text/HTML/attrs), \
         screenshots, JavaScript evaluation, and browser storage operations. Actions chain across calls \
         within the same session (session_create → open → click → fill all operate on the same page). \
         Uses selector-based element targeting. Configure BROWSERLESS_ENABLED=true to use."
            .to_string()
    }
}

fn looks_like_navigation_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn normalize_open_url_params(params_obj: &mut Map<String, Value>) -> Option<String> {
    let has_url = params_obj
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some();

    if has_url {
        return None;
    }

    let normalized = ["selector", "ref"].iter().find_map(|key| {
        params_obj
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| looks_like_navigation_url(value))
            .map(|value| ((*key).to_string(), value.to_string()))
    });

    if let Some((source_field, url)) = normalized {
        params_obj.insert("url".to_string(), Value::String(url));
        return Some(format!(
            "Normalized action=open URL from '{}' to 'url'.",
            source_field
        ));
    }

    None
}

fn normalize_value_params(action: &str, params_obj: &mut Map<String, Value>) -> Option<String> {
    if !matches!(action, "fill" | "type" | "select") {
        return None;
    }

    let has_value = params_obj
        .get("value")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_some();
    if has_value {
        return None;
    }

    let normalized = ["text", "input", "content"].iter().find_map(|key| {
        params_obj
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| ((*key).to_string(), v.to_string()))
    });

    if let Some((source_field, value)) = normalized {
        params_obj.insert("value".to_string(), Value::String(value));
        return Some(format!(
            "Normalized action={} payload from '{}' to 'value'.",
            action, source_field
        ));
    }

    None
}

fn strip_null_fields(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let null_keys: Vec<String> = map
                .iter()
                .filter_map(|(key, value)| value.is_null().then_some(key.clone()))
                .collect();

            for key in null_keys {
                map.remove(&key);
            }

            for value in map.values_mut() {
                strip_null_fields(value);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_null_fields(item);
            }
        }
        _ => {}
    }
}

fn strip_top_level_null_fields(params_obj: &mut Map<String, Value>) -> Vec<String> {
    let removed: Vec<String> = params_obj
        .iter()
        .filter_map(|(key, value)| value.is_null().then_some(key.clone()))
        .collect();

    for key in &removed {
        params_obj.remove(key);
    }

    for value in params_obj.values_mut() {
        strip_null_fields(value);
    }

    removed
}

fn execute_inner(raw_params: &str) -> String {
    if raw_params.len() > MAX_PARAMS_BYTES {
        return error_envelope(
            None,
            None,
            StructuredError::new(
                ERR_INVALID_PARAMS,
                format!("Parameter payload exceeds {} bytes limit", MAX_PARAMS_BYTES),
            )
            .with_hint("Reduce payload size or move large inputs to backend artifacts."),
            None,
        );
    }

    let params: Value = match serde_json::from_str(raw_params) {
        Ok(v) => v,
        Err(err) => {
            return error_envelope(
                None,
                None,
                StructuredError::new(
                    ERR_INVALID_PARAMS,
                    format!("Invalid JSON parameters: {err}"),
                )
                .with_hint("Provide a valid JSON object with at least an 'action' field."),
                None,
            );
        }
    };

    let Some(mut params_obj) = params.as_object().cloned() else {
        return error_envelope(
            None,
            None,
            StructuredError::new(ERR_INVALID_PARAMS, "Parameters must be a JSON object")
                .with_hint("Expected shape: { \"action\": \"...\", ... }"),
            None,
        );
    };

    let removed_null_fields = strip_top_level_null_fields(&mut params_obj);
    let mut normalization_notes: Vec<String> = Vec::new();
    if !removed_null_fields.is_empty() {
        normalization_notes.push(format!(
            "Ignored null fields: {}.",
            removed_null_fields.join(", ")
        ));
    }

    let raw_action = match params_obj.get("action").and_then(Value::as_str) {
        Some(action) if !action.trim().is_empty() => action.to_string(),
        _ => {
            return error_envelope(
                None,
                extract_optional_session_id(&params_obj),
                StructuredError::new(ERR_INVALID_ACTION, "Missing required 'action' string")
                    .with_hint(
                        "Set action to a canonical command like 'open', 'snapshot', or 'click'.",
                    )
                    .with_details(json!({"allowed_actions": CANONICAL_ACTIONS})),
                None,
            );
        }
    };

    let Some(action) = normalize_action(&raw_action) else {
        return error_envelope(
            Some(raw_action.trim()),
            extract_optional_session_id(&params_obj),
            StructuredError::new(
                ERR_INVALID_ACTION,
                format!("Unknown action '{raw_action}'"),
            )
            .with_hint("Use one of the canonical actions or supported aliases (goto/navigate -> open).")
            .with_details(json!({"allowed_actions": CANONICAL_ACTIONS})),
            None,
        );
    };

    if action == "open" {
        if let Some(note) = normalize_open_url_params(&mut params_obj) {
            normalization_notes.push(note);
        }
    }

    if let Some(note) = normalize_value_params(action, &mut params_obj) {
        normalization_notes.push(note);
    }

    let normalized_params = Value::Object(params_obj.clone());

    if let Err(err) = validate_action_params(action, &normalized_params) {
        return error_envelope(
            Some(action),
            extract_optional_session_id(&params_obj),
            err,
            None,
        );
    }

    let backend_url = match resolve_backend_url(&params_obj) {
        Ok(url) => url,
        Err(err) => {
            return error_envelope(
                Some(action),
                extract_optional_session_id(&params_obj),
                err,
                None,
            );
        }
    };

    let timeout_ms = resolve_timeout_ms(action, &params_obj);

    match dispatch_with_retries(action, &normalized_params, &backend_url, timeout_ms) {
        Ok(success) => {
            let mut warnings = success.warnings;
            if let Some(note) = alias_note(&raw_action, action) {
                warnings.push(note);
            }
            warnings.extend(normalization_notes.clone());

            let fallback_session_id = extract_optional_session_id(&params_obj);
            let session_id = success
                .session_id
                .as_deref()
                .or(fallback_session_id.as_deref());

            success_envelope(
                action,
                session_id,
                success.snapshot_id.as_deref(),
                success.data,
                json!({
                    "contract_version": CONTRACT_VERSION,
                    "attempts": success.attempts,
                    "timeout_ms": timeout_ms,
                    "backend_status": success.backend_status,
                    "warnings": warnings,
                }),
            )
        }
        Err(failure) => {
            let mut meta = json!({
                "contract_version": CONTRACT_VERSION,
                "attempts": failure.attempts,
                "timeout_ms": timeout_ms,
            });
            if !normalization_notes.is_empty() {
                if let Some(obj) = meta.as_object_mut() {
                    obj.insert(
                        "normalization_notes".to_string(),
                        json!(normalization_notes),
                    );
                }
            }

            error_envelope(
                Some(action),
                extract_optional_session_id(&params_obj),
                failure.error,
                Some(meta),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_json(s: &str) -> Value {
        serde_json::from_str(s).expect("valid json")
    }

    #[test]
    fn test_unknown_action_returns_invalid_action() {
        let output = execute_inner(r#"{"action":"does_not_exist"}"#);
        let parsed = parse_json(&output);
        assert_eq!(parsed["ok"], Value::Bool(false));
        assert_eq!(
            parsed["error"]["code"],
            Value::String(ERR_INVALID_ACTION.into())
        );
    }

    #[test]
    fn test_missing_action_returns_error() {
        let output = execute_inner(r#"{"session_id":"s1"}"#);
        let parsed = parse_json(&output);
        assert_eq!(parsed["ok"], Value::Bool(false));
        assert_eq!(
            parsed["error"]["code"],
            Value::String(ERR_INVALID_ACTION.into())
        );
    }

    #[test]
    fn test_invalid_json_returns_error() {
        let output = execute_inner("not json at all");
        let parsed = parse_json(&output);
        assert_eq!(parsed["ok"], Value::Bool(false));
        assert_eq!(
            parsed["error"]["code"],
            Value::String(ERR_INVALID_PARAMS.into())
        );
    }

    #[test]
    fn test_non_object_params_returns_error() {
        let output = execute_inner(r#""just a string""#);
        let parsed = parse_json(&output);
        assert_eq!(parsed["ok"], Value::Bool(false));
        assert_eq!(
            parsed["error"]["code"],
            Value::String(ERR_INVALID_PARAMS.into())
        );
    }

    #[test]
    fn test_open_without_url_returns_validation_error() {
        let output = execute_inner(r#"{"action":"open","session_id":"s1"}"#);
        let parsed = parse_json(&output);
        assert_eq!(parsed["ok"], Value::Bool(false));
        assert_eq!(
            parsed["error"]["code"],
            Value::String(ERR_INVALID_PARAMS.into())
        );
    }

    #[test]
    fn test_click_without_selector_returns_validation_error() {
        let output = execute_inner(r#"{"action":"click","session_id":"s1"}"#);
        let parsed = parse_json(&output);
        assert_eq!(parsed["ok"], Value::Bool(false));
        assert_eq!(
            parsed["error"]["code"],
            Value::String(ERR_INVALID_PARAMS.into())
        );
    }

    #[test]
    fn test_fill_without_value_returns_validation_error() {
        let output = execute_inner(r#"{"action":"fill","session_id":"s1","selector":"input"}"#);
        let parsed = parse_json(&output);
        assert_eq!(parsed["ok"], Value::Bool(false));
        assert_eq!(
            parsed["error"]["code"],
            Value::String(ERR_INVALID_PARAMS.into())
        );
    }

    #[test]
    fn test_eval_without_script_returns_validation_error() {
        let output = execute_inner(r#"{"action":"eval","session_id":"s1"}"#);
        let parsed = parse_json(&output);
        assert_eq!(parsed["ok"], Value::Bool(false));
        assert_eq!(
            parsed["error"]["code"],
            Value::String(ERR_INVALID_PARAMS.into())
        );
    }

    #[test]
    fn test_alias_normalization() {
        // Test normalization without needing dispatch (which requires host WS runtime)
        assert_eq!(normalize_action("goto"), Some("open"));
        assert_eq!(normalize_action("navigate"), Some("open"));
        assert_eq!(normalize_action("take_screenshot"), Some("screenshot"));
        assert_eq!(normalize_action("evaluate"), Some("eval"));
        assert_eq!(normalize_action("nonexistent"), None);
    }

    #[test]
    fn test_open_normalizes_selector_url() {
        let mut params = serde_json::Map::from_iter([
            ("action".to_string(), Value::String("open".to_string())),
            (
                "selector".to_string(),
                Value::String("https://www.wikipedia.org".to_string()),
            ),
        ]);

        let note = normalize_open_url_params(&mut params);

        assert_eq!(
            params.get("url"),
            Some(&Value::String("https://www.wikipedia.org".to_string()))
        );
        assert!(note.is_some());
    }

    #[test]
    fn test_open_normalizes_ref_url() {
        let mut params = serde_json::Map::from_iter([
            ("action".to_string(), Value::String("open".to_string())),
            (
                "ref".to_string(),
                Value::String("https://example.com".to_string()),
            ),
        ]);

        let note = normalize_open_url_params(&mut params);

        assert_eq!(
            params.get("url"),
            Some(&Value::String("https://example.com".to_string()))
        );
        assert!(note.is_some());
    }

    #[test]
    fn test_open_does_not_override_existing_url() {
        let mut params = serde_json::Map::from_iter([
            ("action".to_string(), Value::String("open".to_string())),
            (
                "url".to_string(),
                Value::String("https://already-set.example".to_string()),
            ),
            (
                "selector".to_string(),
                Value::String("https://wrong-source.example".to_string()),
            ),
        ]);

        let note = normalize_open_url_params(&mut params);

        assert_eq!(
            params.get("url"),
            Some(&Value::String("https://already-set.example".to_string()))
        );
        assert!(note.is_none());
    }

    #[test]
    fn test_fill_normalizes_text_to_value() {
        let mut params = serde_json::Map::from_iter([
            ("action".to_string(), Value::String("fill".to_string())),
            ("selector".to_string(), Value::String("#email".to_string())),
            (
                "text".to_string(),
                Value::String("user@example.com".to_string()),
            ),
        ]);

        let note = normalize_value_params("fill", &mut params);

        assert_eq!(
            params.get("value"),
            Some(&Value::String("user@example.com".to_string()))
        );
        assert!(note.is_some());
    }

    #[test]
    fn test_type_normalizes_input_to_value() {
        let mut params = serde_json::Map::from_iter([
            ("action".to_string(), Value::String("type".to_string())),
            (
                "selector".to_string(),
                Value::String("#password".to_string()),
            ),
            ("input".to_string(), Value::String("secret".to_string())),
        ]);

        let note = normalize_value_params("type", &mut params);

        assert_eq!(
            params.get("value"),
            Some(&Value::String("secret".to_string()))
        );
        assert!(note.is_some());
    }

    #[test]
    fn test_value_normalization_does_not_override_existing_value() {
        let mut params = serde_json::Map::from_iter([
            ("action".to_string(), Value::String("fill".to_string())),
            ("selector".to_string(), Value::String("#field".to_string())),
            (
                "value".to_string(),
                Value::String("authoritative".to_string()),
            ),
            ("text".to_string(), Value::String("ignored".to_string())),
        ]);

        let note = normalize_value_params("fill", &mut params);

        assert_eq!(
            params.get("value"),
            Some(&Value::String("authoritative".to_string()))
        );
        assert!(note.is_none());
    }

    #[test]
    fn test_strip_top_level_null_fields_removes_nulls() {
        let mut params = serde_json::Map::from_iter([
            ("action".to_string(), Value::String("snapshot".to_string())),
            ("selector".to_string(), Value::Null),
            ("ref".to_string(), Value::Null),
            (
                "session_id".to_string(),
                Value::String("session-1".to_string()),
            ),
        ]);

        let removed = strip_top_level_null_fields(&mut params);

        assert_eq!(removed, vec!["ref".to_string(), "selector".to_string()]);
        assert!(params.get("selector").is_none());
        assert!(params.get("ref").is_none());
        assert_eq!(
            params.get("session_id"),
            Some(&Value::String("session-1".to_string()))
        );
    }

    #[test]
    fn test_strip_top_level_null_fields_strips_nested_nulls() {
        let mut params = serde_json::Map::from_iter([
            (
                "action".to_string(),
                Value::String("cookies_set_batch".to_string()),
            ),
            (
                "cookies".to_string(),
                json!([
                    {"name": "a", "value": "1", "sameSite": null},
                    {"name": "b", "value": "2", "secure": true}
                ]),
            ),
        ]);

        let removed = strip_top_level_null_fields(&mut params);

        assert!(removed.is_empty());
        let cookies = params
            .get("cookies")
            .and_then(Value::as_array)
            .expect("cookies array");
        assert!(cookies[0].get("sameSite").is_none());
        assert_eq!(cookies[1].get("secure"), Some(&Value::Bool(true)));
    }

    #[test]
    fn test_strip_top_level_null_fields_removes_nested_object_nulls() {
        let mut params = serde_json::Map::from_iter([
            ("action".to_string(), Value::String("eval".to_string())),
            (
                "options".to_string(),
                json!({
                    "timeout": 1000,
                    "frame": null,
                    "meta": {"tag": "x", "hint": null}
                }),
            ),
        ]);

        let removed = strip_top_level_null_fields(&mut params);

        assert!(removed.is_empty());
        let options = params
            .get("options")
            .and_then(Value::as_object)
            .expect("options object");
        assert!(options.get("frame").is_none());
        let meta = options
            .get("meta")
            .and_then(Value::as_object)
            .expect("meta object");
        assert!(meta.get("hint").is_none());
        assert_eq!(meta.get("tag"), Some(&Value::String("x".to_string())));
    }

    #[test]
    fn test_is_session_action_true_cases() {
        use crate::session::is_session_action;
        assert!(is_session_action("session_create"));
        assert!(is_session_action("session_list"));
        assert!(is_session_action("session_resume"));
        assert!(is_session_action("session_close"));
        assert!(is_session_action("state_save"));
        assert!(is_session_action("state_load"));
    }

    #[test]
    fn test_is_session_action_false_cases() {
        use crate::session::is_session_action;
        assert!(!is_session_action("open"));
        assert!(!is_session_action("click"));
        assert!(!is_session_action("snapshot"));
        assert!(!is_session_action("eval"));
        assert!(!is_session_action("screenshot"));
    }
}

export!(BrowserUseTool);
