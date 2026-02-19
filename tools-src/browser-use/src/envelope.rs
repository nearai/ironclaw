use serde_json::{Map, Value};

use crate::constants::MAX_ERROR_MESSAGE_BYTES;
use crate::error::StructuredError;

pub fn success_envelope(
    action: &str,
    session_id: Option<&str>,
    snapshot_id: Option<&str>,
    data: Value,
    meta: Value,
) -> String {
    let mut root = Map::new();
    root.insert("ok".to_string(), Value::Bool(true));
    root.insert("action".to_string(), Value::String(action.to_string()));
    if let Some(sid) = session_id {
        root.insert("session_id".to_string(), Value::String(sid.to_string()));
    }
    if let Some(snap) = snapshot_id {
        root.insert("snapshot_id".to_string(), Value::String(snap.to_string()));
    }
    root.insert("data".to_string(), data);
    root.insert("meta".to_string(), meta);

    Value::Object(root).to_string()
}

pub fn error_envelope(
    action: Option<&str>,
    session_id: Option<String>,
    err: StructuredError,
    meta: Option<Value>,
) -> String {
    let mut error_obj = Map::new();
    error_obj.insert("code".to_string(), Value::String(err.code.to_string()));
    error_obj.insert(
        "message".to_string(),
        Value::String(truncate_message(&err.message)),
    );
    error_obj.insert("retryable".to_string(), Value::Bool(err.retryable));
    if let Some(hint) = err.hint {
        error_obj.insert("hint".to_string(), Value::String(hint));
    }
    if let Some(details) = err.details {
        error_obj.insert("details".to_string(), details);
    }

    let mut root = Map::new();
    root.insert("ok".to_string(), Value::Bool(false));
    root.insert(
        "action".to_string(),
        Value::String(action.unwrap_or("unknown").to_string()),
    );
    if let Some(sid) = session_id {
        root.insert("session_id".to_string(), Value::String(sid));
    }
    root.insert("error".to_string(), Value::Object(error_obj));
    if let Some(meta) = meta {
        root.insert("meta".to_string(), meta);
    }

    Value::Object(root).to_string()
}

pub fn truncate_message(message: &str) -> String {
    if message.len() <= MAX_ERROR_MESSAGE_BYTES {
        return message.to_string();
    }

    let mut truncated = message
        .chars()
        .take(MAX_ERROR_MESSAGE_BYTES.saturating_sub(3))
        .collect::<String>();
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::constants::ERR_INVALID_REF;

    fn parse_json(s: &str) -> Value {
        serde_json::from_str(s).expect("valid json")
    }

    #[test]
    fn test_success_envelope_contract_shape() {
        let output = success_envelope(
            "snapshot",
            Some("s-1"),
            Some("snap-1"),
            json!({"refs": []}),
            json!({"attempts": 1}),
        );
        let parsed = parse_json(&output);

        assert_eq!(parsed["ok"], Value::Bool(true));
        assert_eq!(parsed["action"], Value::String("snapshot".into()));
        assert_eq!(parsed["session_id"], Value::String("s-1".into()));
        assert_eq!(parsed["snapshot_id"], Value::String("snap-1".into()));
        assert!(parsed.get("data").is_some());
        assert!(parsed.get("meta").is_some());
    }

    #[test]
    fn test_error_envelope_contract_shape() {
        let output = error_envelope(
            Some("click"),
            Some("s-1".to_string()),
            StructuredError::new(ERR_INVALID_REF, "bad ref"),
            None,
        );

        let parsed = parse_json(&output);
        assert_eq!(parsed["ok"], Value::Bool(false));
        assert_eq!(parsed["action"], Value::String("click".into()));
        assert_eq!(parsed["session_id"], Value::String("s-1".into()));
        assert_eq!(
            parsed["error"]["code"],
            Value::String(ERR_INVALID_REF.into())
        );
        assert!(parsed["error"].get("retryable").is_some());
    }
}
