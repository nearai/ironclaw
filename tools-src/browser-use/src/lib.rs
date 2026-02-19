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

use serde_json::{json, Value};

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
                "session_id": {
                    "type": "string",
                    "description": "Browser session identifier. Required for all actions except session_create/session_list."
                },
                "ref": {
                    "type": "string",
                    "description": "Deterministic snapshot ref, format @eN (e.g. @e12)."
                },
                "selector": {
                    "type": "string",
                    "description": "Semantic selector when ref is unavailable."
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

    let Some(params_obj) = params.as_object() else {
        return error_envelope(
            None,
            None,
            StructuredError::new(ERR_INVALID_PARAMS, "Parameters must be a JSON object")
                .with_hint("Expected shape: { \"action\": \"...\", ... }"),
            None,
        );
    };

    let raw_action = match params_obj.get("action").and_then(Value::as_str) {
        Some(action) if !action.trim().is_empty() => action,
        _ => {
            return error_envelope(
                None,
                extract_optional_session_id(params_obj),
                StructuredError::new(ERR_INVALID_ACTION, "Missing required 'action' string")
                    .with_hint(
                        "Set action to a canonical command like 'open', 'snapshot', or 'click'.",
                    )
                    .with_details(json!({"allowed_actions": CANONICAL_ACTIONS})),
                None,
            );
        }
    };

    let Some(action) = normalize_action(raw_action) else {
        return error_envelope(
            Some(raw_action.trim()),
            extract_optional_session_id(params_obj),
            StructuredError::new(
                ERR_INVALID_ACTION,
                format!("Unknown action '{raw_action}'"),
            )
            .with_hint("Use one of the canonical actions or supported aliases (goto/navigate -> open).")
            .with_details(json!({"allowed_actions": CANONICAL_ACTIONS})),
            None,
        );
    };

    if let Err(err) = validate_action_params(action, &params) {
        return error_envelope(
            Some(action),
            extract_optional_session_id(params_obj),
            err,
            None,
        );
    }

    let backend_url = match resolve_backend_url(params_obj) {
        Ok(url) => url,
        Err(err) => {
            return error_envelope(
                Some(action),
                extract_optional_session_id(params_obj),
                err,
                None,
            );
        }
    };

    let timeout_ms = resolve_timeout_ms(action, params_obj);

    match dispatch_with_retries(action, &params, &backend_url, timeout_ms) {
        Ok(success) => {
            let mut warnings = success.warnings;
            if let Some(note) = alias_note(raw_action, action) {
                warnings.push(note);
            }

            let fallback_session_id = extract_optional_session_id(params_obj);
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
        Err(failure) => error_envelope(
            Some(action),
            extract_optional_session_id(params_obj),
            failure.error,
            Some(json!({
                "contract_version": CONTRACT_VERSION,
                "attempts": failure.attempts,
                "timeout_ms": timeout_ms,
            })),
        ),
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
