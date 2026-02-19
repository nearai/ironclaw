use serde_json::{json, Map, Value};

use crate::constants::*;
use crate::error::StructuredError;

pub fn validate_action_params(action: &str, params: &Value) -> Result<(), StructuredError> {
    let Some(obj) = params.as_object() else {
        return Err(StructuredError::new(
            ERR_INVALID_PARAMS,
            "Parameters must be a JSON object",
        ));
    };

    if let Some(timeout) = obj.get("timeout_ms") {
        if !timeout.is_u64() {
            return Err(StructuredError::new(
                ERR_INVALID_PARAMS,
                "Field 'timeout_ms' must be an integer",
            ));
        }
    }

    match action {
        "open" => {
            let url = require_non_empty_string(obj, "url")?;
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err(StructuredError::new(
                    ERR_INVALID_PARAMS,
                    "Field 'url' must start with http:// or https://",
                ));
            }
        }

        "snapshot" => {
            if let Some(mode) = obj.get("mode").and_then(Value::as_str) {
                match mode {
                    "full" | "interactive-only" | "compact" => {}
                    _ => {
                        return Err(StructuredError::new(
                            ERR_INVALID_PARAMS,
                            "Field 'mode' must be one of: full, interactive-only, compact",
                        ));
                    }
                }
            }

            if let Some(depth) = obj.get("depth") {
                let valid = depth.as_u64().map(|d| d > 0 && d <= 64).unwrap_or(false);
                if !valid {
                    return Err(StructuredError::new(
                        ERR_INVALID_PARAMS,
                        "Field 'depth' must be an integer between 1 and 64",
                    ));
                }
            }

            if obj.get("selector").is_some() {
                validate_selector_field(obj, "selector")?;
            }
        }

        "click" | "dblclick" | "focus" | "hover" | "check" | "uncheck" | "scroll_into_view"
        | "get_text" | "get_html" | "get_value" | "get_count" | "get_box" => {
            validate_single_target(obj, "ref", "selector")?;
        }

        "fill" => {
            validate_single_target(obj, "ref", "selector")?;
            require_non_empty_string(obj, "value")?;
        }

        "type" => {
            validate_single_target(obj, "ref", "selector")?;
            require_non_empty_string(obj, "value")?;
        }

        "select" => {
            validate_single_target(obj, "ref", "selector")?;
            require_non_empty_string(obj, "value")?;
        }

        "press" | "keydown" | "keyup" => {
            require_non_empty_string(obj, "key")?;
            validate_optional_target(obj, "ref", "selector")?;
        }

        "scroll" => {
            let has_x = obj.get("x").and_then(Value::as_i64).is_some();
            let has_y = obj.get("y").and_then(Value::as_i64).is_some();
            if !has_x && !has_y {
                return Err(StructuredError::new(
                    ERR_INVALID_PARAMS,
                    "Action 'scroll' requires 'x' and/or 'y'",
                ));
            }
        }

        "drag" => {
            validate_single_target(obj, "source_ref", "source_selector")?;
            validate_single_target(obj, "target_ref", "target_selector")?;
        }

        "upload" => {
            validate_single_target(obj, "ref", "selector")?;
            validate_string_array_field(obj, "files", true)?;
        }

        "wait" => validate_wait_modes(obj)?,

        "get_attr" => {
            validate_single_target(obj, "ref", "selector")?;
            require_non_empty_string(obj, "name")?;
        }

        "screenshot" => {
            if let Some(inline) = obj.get("inline") {
                if !inline.is_boolean() {
                    return Err(StructuredError::new(
                        ERR_INVALID_PARAMS,
                        "Field 'inline' must be boolean",
                    ));
                }
            }

            if let Some(full_page) = obj.get("full_page") {
                if !full_page.is_boolean() {
                    return Err(StructuredError::new(
                        ERR_INVALID_PARAMS,
                        "Field 'full_page' must be boolean",
                    ));
                }
            }
        }

        "session_create" | "session_list" | "back" | "forward" | "reload" | "get_title"
        | "get_url" => {}

        "session_resume" | "session_close" | "state_save" | "state_load" => {
            require_non_empty_string(obj, "session_id")?;
        }

        "pdf" => {
            if let Some(format) = obj.get("format").and_then(|v| v.as_str()) {
                if !["A3", "A4", "A5", "Legal", "Letter", "Tabloid"].contains(&format) {
                    return Err(StructuredError::new(
                        ERR_INVALID_PARAMS,
                        "Field 'format' must be one of: A3, A4, A5, Legal, Letter, Tabloid",
                    ));
                }
            }
        }

        "cookies_get" | "cookies_delete" => {
            require_non_empty_string(obj, "name")?;
        }

        "local_storage_get"
        | "local_storage_delete"
        | "session_storage_get"
        | "session_storage_delete" => {
            require_non_empty_string(obj, "key")?;
        }

        "cookies_set" => {
            require_non_empty_string(obj, "name")?;
            require_non_empty_string(obj, "value")?;
        }

        "cookies_set_batch" => {
            let cookies = obj.get("cookies").and_then(|v| v.as_array());
            match cookies {
                Some(arr) if !arr.is_empty() => {
                    for (i, entry) in arr.iter().enumerate() {
                        let entry_obj = entry.as_object().ok_or_else(|| {
                            StructuredError::new(
                                ERR_INVALID_PARAMS,
                                format!("cookies[{i}] must be a JSON object"),
                            )
                        })?;
                        if entry_obj
                            .get("name")
                            .and_then(|v| v.as_str())
                            .is_none_or(|s| s.is_empty())
                        {
                            return Err(StructuredError::new(
                                ERR_INVALID_PARAMS,
                                format!("cookies[{i}].name is required"),
                            ));
                        }
                        if entry_obj
                            .get("value")
                            .and_then(|v| v.as_str())
                            .is_none_or(|s| s.is_empty())
                        {
                            return Err(StructuredError::new(
                                ERR_INVALID_PARAMS,
                                format!("cookies[{i}].value is required"),
                            ));
                        }
                    }
                }
                _ => {
                    return Err(StructuredError::new(
                        ERR_INVALID_PARAMS,
                        "cookies_set_batch requires a non-empty 'cookies' array",
                    ));
                }
            }
        }

        "local_storage_set" | "session_storage_set" => {
            require_non_empty_string(obj, "key")?;
            require_non_empty_string(obj, "value")?;
        }

        "cookies_list" | "local_storage_list" | "session_storage_list" => {}

        "eval" => {
            let script = require_non_empty_string(obj, "script")?;
            if script.len() > MAX_SCRIPT_BYTES {
                return Err(StructuredError::new(
                    ERR_INVALID_PARAMS,
                    format!("Field 'script' exceeds {} bytes limit", MAX_SCRIPT_BYTES),
                ));
            }
        }

        _ => {
            return Err(StructuredError::new(
                ERR_INVALID_ACTION,
                format!("Unsupported action '{action}'"),
            ));
        }
    }

    Ok(())
}

fn validate_wait_modes(obj: &Map<String, Value>) -> Result<(), StructuredError> {
    let mode_keys = [
        "ms",
        "ref",
        "selector",
        "text",
        "url_pattern",
        "load_state",
        "js_condition",
    ];

    let set_modes: Vec<&str> = mode_keys
        .iter()
        .copied()
        .filter(|key| obj.get(*key).is_some())
        .collect();

    if set_modes.len() != 1 {
        return Err(StructuredError::new(
            ERR_INVALID_PARAMS,
            "Action 'wait' requires exactly one wait mode: ms, ref, selector, text, url_pattern, load_state, or js_condition",
        ));
    }

    if let Some(ms) = obj.get("ms") {
        let valid = ms.as_u64().map(|m| m > 0 && m <= 120_000).unwrap_or(false);
        if !valid {
            return Err(StructuredError::new(
                ERR_INVALID_PARAMS,
                "Field 'ms' must be an integer between 1 and 120000",
            ));
        }
    }

    if obj.get("ref").is_some() {
        validate_ref_field(obj, "ref")?;
    }

    if obj.get("selector").is_some() {
        validate_selector_field(obj, "selector")?;
    }

    if obj.get("text").is_some() {
        require_non_empty_string(obj, "text")?;
    }

    if obj.get("url_pattern").is_some() {
        require_non_empty_string(obj, "url_pattern")?;
    }

    if let Some(load_state) = obj.get("load_state").and_then(Value::as_str) {
        match load_state {
            "load" | "domcontentloaded" | "networkidle" => {}
            _ => {
                return Err(StructuredError::new(
                    ERR_INVALID_PARAMS,
                    "Field 'load_state' must be one of: load, domcontentloaded, networkidle",
                ));
            }
        }
    }

    if obj.get("js_condition").is_some() {
        require_non_empty_string(obj, "js_condition")?;
    }

    Ok(())
}

fn validate_single_target(
    obj: &Map<String, Value>,
    ref_key: &str,
    selector_key: &str,
) -> Result<(), StructuredError> {
    let has_ref = obj.get(ref_key).is_some();
    let has_selector = obj.get(selector_key).is_some();

    if !has_ref && !has_selector {
        return Err(StructuredError::new(
            ERR_INVALID_PARAMS,
            format!("Provide '{selector_key}' for element targeting"),
        )
        .with_hint("Refs require a prior snapshot; use selector for direct element targeting."));
    }

    if has_selector {
        validate_selector_field(obj, selector_key)?;
    } else if has_ref {
        return Err(StructuredError::new(
            ERR_NOT_IMPLEMENTED,
            format!("'{ref_key}' requires WebSocket CDP connection for snapshot-based targeting"),
        )
        .with_hint("Use 'selector' for REST API element interactions."));
    }

    Ok(())
}

fn validate_optional_target(
    obj: &Map<String, Value>,
    ref_key: &str,
    selector_key: &str,
) -> Result<(), StructuredError> {
    let has_ref = obj.get(ref_key).is_some();
    let has_selector = obj.get(selector_key).is_some();

    if has_ref && has_selector {
        return Err(StructuredError::new(
            ERR_INVALID_PARAMS,
            format!("Provide at most one of '{ref_key}' or '{selector_key}'"),
        ));
    }

    if has_ref {
        validate_ref_field(obj, ref_key)?;
    }

    if has_selector {
        validate_selector_field(obj, selector_key)?;
    }

    Ok(())
}

fn validate_ref_field(obj: &Map<String, Value>, key: &str) -> Result<(), StructuredError> {
    let value = require_non_empty_string(obj, key)?;
    if !is_valid_ref(value) {
        return Err(StructuredError::new(
            ERR_INVALID_REF,
            format!("Field '{key}' must match @eN (example: @e12)"),
        )
        .with_hint("Re-run snapshot and use one of the returned refs.")
        .with_details(json!({
            "ref_error": "malformed_ref",
            "field": key,
            "value": value,
        })));
    }

    Ok(())
}

pub fn validate_selector_field(obj: &Map<String, Value>, key: &str) -> Result<(), StructuredError> {
    let selector = require_non_empty_string(obj, key)?;

    if selector.len() > MAX_SELECTOR_BYTES {
        return Err(StructuredError::new(
            ERR_INVALID_SELECTOR,
            format!("Field '{key}' exceeds {MAX_SELECTOR_BYTES} bytes"),
        ));
    }

    if selector.contains('\0') {
        return Err(StructuredError::new(
            ERR_INVALID_SELECTOR,
            format!("Field '{key}' contains invalid null byte"),
        ));
    }

    Ok(())
}

fn validate_string_array_field(
    obj: &Map<String, Value>,
    key: &str,
    require_non_empty: bool,
) -> Result<(), StructuredError> {
    let Some(value) = obj.get(key) else {
        return Err(StructuredError::new(
            ERR_INVALID_PARAMS,
            format!("Missing required field '{key}'"),
        ));
    };

    let Some(items) = value.as_array() else {
        return Err(StructuredError::new(
            ERR_INVALID_PARAMS,
            format!("Field '{key}' must be an array"),
        ));
    };

    if require_non_empty && items.is_empty() {
        return Err(StructuredError::new(
            ERR_INVALID_PARAMS,
            format!("Field '{key}' must not be empty"),
        ));
    }

    for item in items {
        match item.as_str().map(str::trim) {
            Some(v) if !v.is_empty() => {}
            _ => {
                return Err(StructuredError::new(
                    ERR_INVALID_PARAMS,
                    format!("All entries in '{key}' must be non-empty strings"),
                ));
            }
        }
    }

    Ok(())
}

pub fn require_non_empty_string<'a>(
    obj: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, StructuredError> {
    match obj.get(key).and_then(Value::as_str).map(str::trim) {
        Some(value) if !value.is_empty() => Ok(value),
        Some(_) => Err(StructuredError::new(
            ERR_INVALID_PARAMS,
            format!("Field '{key}' must not be empty"),
        )),
        None => Err(StructuredError::new(
            ERR_INVALID_PARAMS,
            format!("Missing required field '{key}'"),
        )),
    }
}

pub fn is_valid_ref(reference: &str) -> bool {
    let bytes = reference.as_bytes();
    if bytes.len() < 3 {
        return false;
    }
    bytes[0] == b'@' && bytes[1] == b'e' && bytes[2..].iter().all(u8::is_ascii_digit)
}

pub fn resolve_timeout_ms(action: &str, params: &Map<String, Value>) -> u32 {
    let default_ms = match action {
        "wait" | "snapshot" | "screenshot" => 30_000,
        "upload" => 45_000,
        "eval" => 20_000,
        _ => 15_000,
    };

    params
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .map(|t| t.clamp(1, MAX_ACTION_TIMEOUT_MS as u64) as u32)
        .unwrap_or(default_ms)
}

pub fn resolve_backend_url(params: &Map<String, Value>) -> Result<String, StructuredError> {
    let url = params
        .get("backend_url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BROWSERLESS_URL);

    let is_local = url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost");

    if !is_local {
        return Err(StructuredError::new(
            ERR_POLICY_BLOCKED,
            "backend_url must target localhost (Browserless sidecar)",
        )
        .with_hint("Configure BROWSERLESS_ENABLED=true and use http://localhost:9222"));
    }

    Ok(url.to_string())
}

pub fn extract_optional_session_id(params: &Map<String, Value>) -> Option<String> {
    params
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wait_validation_requires_single_mode() {
        let err = validate_action_params(
            "wait",
            &json!({"action": "wait", "session_id": "s1", "ms": 100, "text": "ready"}),
        )
        .expect_err("must fail for multiple modes");
        assert_eq!(err.code, ERR_INVALID_PARAMS);
    }

    #[test]
    fn test_ref_not_supported_in_rest_mode() {
        let err = validate_action_params(
            "click",
            &json!({"action": "click", "session_id": "s1", "ref": "@e1"}),
        )
        .expect_err("must fail ref not supported");
        assert_eq!(err.code, ERR_NOT_IMPLEMENTED);
    }

    #[test]
    fn test_type_validation_uses_value_field() {
        let err = validate_action_params(
            "type",
            &json!({"action": "type", "session_id": "s1", "selector": "input", "text": "abc"}),
        )
        .expect_err("must fail missing value");
        assert_eq!(err.code, ERR_INVALID_PARAMS);
    }

    #[test]
    fn test_get_attr_validation_uses_name_field() {
        let err = validate_action_params(
            "get_attr",
            &json!({"action": "get_attr", "session_id": "s1", "selector": "a.link", "attr": "href"}),
        )
        .expect_err("must fail missing name");
        assert_eq!(err.code, ERR_INVALID_PARAMS);
    }

    #[test]
    fn test_cookies_validation_uses_name_field() {
        let err = validate_action_params(
            "cookies_get",
            &json!({"action": "cookies_get", "session_id": "s1", "key": "token"}),
        )
        .expect_err("must fail missing name");
        assert_eq!(err.code, ERR_INVALID_PARAMS);
    }

    #[test]
    fn test_selector_validation_uses_invalid_selector_code() {
        let err = validate_action_params(
            "click",
            &json!({"action": "click", "session_id": "s1", "selector": "bad\u{0000}selector"}),
        )
        .expect_err("must fail invalid selector");
        assert_eq!(err.code, ERR_INVALID_SELECTOR);
    }

    #[test]
    fn test_eval_validation_enforces_script_limit() {
        let script = "a".repeat(MAX_SCRIPT_BYTES + 1);
        let err = validate_action_params(
            "eval",
            &json!({"action": "eval", "session_id": "s1", "script": script}),
        )
        .expect_err("must fail script too large");
        assert_eq!(err.code, ERR_INVALID_PARAMS);
    }

    #[test]
    fn test_backend_url_policy_is_localhost_only() {
        let err = resolve_backend_url(&Map::from_iter([(
            "backend_url".to_string(),
            Value::String("https://evil.example.com/v1/browser/dispatch".to_string()),
        )]))
        .expect_err("must fail non-local URL");
        assert_eq!(err.code, ERR_POLICY_BLOCKED);
    }

    #[test]
    fn test_timeout_resolution_bounds_override() {
        let params = Map::from_iter([("timeout_ms".to_string(), json!(999_999))]);
        assert_eq!(resolve_timeout_ms("open", &params), MAX_ACTION_TIMEOUT_MS);
    }

    #[test]
    fn test_session_create_validation_passes_without_session_id() {
        let result = validate_action_params("session_create", &json!({"action": "session_create"}));
        assert!(result.is_ok());
    }

    #[test]
    fn test_session_resume_validation_requires_session_id() {
        let result = validate_action_params("session_resume", &json!({"action": "session_resume"}));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ERR_INVALID_PARAMS);
    }

    #[test]
    fn test_session_close_validation_requires_session_id() {
        let result = validate_action_params("session_close", &json!({"action": "session_close"}));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ERR_INVALID_PARAMS);
    }

    #[test]
    fn test_state_save_validation_requires_session_id() {
        let result = validate_action_params("state_save", &json!({"action": "state_save"}));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ERR_INVALID_PARAMS);
    }

    #[test]
    fn test_state_load_validation_requires_session_id() {
        let result = validate_action_params("state_load", &json!({"action": "state_load"}));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ERR_INVALID_PARAMS);
    }

    #[test]
    fn test_cookies_set_batch_requires_cookies_array() {
        let result = validate_action_params(
            "cookies_set_batch",
            &json!({"action": "cookies_set_batch", "session_id": "s1"}),
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ERR_INVALID_PARAMS);
    }

    #[test]
    fn test_cookies_set_batch_requires_nonempty_array() {
        let result = validate_action_params(
            "cookies_set_batch",
            &json!({"action": "cookies_set_batch", "session_id": "s1", "cookies": []}),
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, ERR_INVALID_PARAMS);
    }

    #[test]
    fn test_cookies_set_batch_validates_entry_fields() {
        let result = validate_action_params(
            "cookies_set_batch",
            &json!({"action": "cookies_set_batch", "session_id": "s1", "cookies": [{"name": "a"}]}),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_cookies_set_batch_passes_valid_input() {
        let result = validate_action_params(
            "cookies_set_batch",
            &json!({
                "action": "cookies_set_batch",
                "session_id": "s1",
                "cookies": [
                    {"name": "a", "value": "1", "domain": ".example.com"},
                    {"name": "b", "value": "2", "domain": ".example.com"}
                ]
            }),
        );
        assert!(result.is_ok());
    }
}
