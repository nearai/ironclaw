//! Shared utility functions used across the codebase.

use crate::llm::{ChatMessage, Role};
use serde_json::{Map, Value};

/// Find the largest valid UTF-8 char boundary at or before `pos`.
///
/// Polyfill for `str::floor_char_boundary` (nightly-only). Use when
/// truncating strings by byte position to avoid panicking on multi-byte
/// characters.
pub fn floor_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut i = pos;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Ensure the last message in `messages` is a user-role message.
///
/// NEAR AI rejects conversations that don't end with a user message;
/// Claude 4.6 rejects assistant prefill. Call this before any LLM
/// completion request to satisfy both requirements.
pub fn ensure_ends_with_user_message(messages: &mut Vec<ChatMessage>) {
    if !matches!(messages.last(), Some(m) if m.role == Role::User) {
        messages.push(ChatMessage::user("Continue."));
    }
}

/// Recursively sort JSON object keys for deterministic comparison/hashing.
///
/// Arrays preserve order, objects get sorted keys, scalars pass through.
pub fn canonicalize_json_value(value: Value) -> Value {
    match value {
        Value::Array(items) => {
            Value::Array(items.into_iter().map(canonicalize_json_value).collect())
        }
        Value::Object(obj) => {
            let mut keys: Vec<String> = obj.keys().cloned().collect();
            keys.sort();
            let mut canonical = Map::new();
            for key in keys {
                if let Some(value) = obj.get(&key) {
                    canonical.insert(key, canonicalize_json_value(value.clone()));
                }
            }
            Value::Object(canonical)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use crate::llm::ChatMessage;
    use crate::util::{
        canonicalize_json_value, ensure_ends_with_user_message, floor_char_boundary,
    };

    // ── canonicalize_json_value ──

    #[test]
    fn canonicalize_sorts_object_keys() {
        let input = serde_json::json!({"z": 1, "a": 2, "m": 3});
        let result = canonicalize_json_value(input);
        let keys: Vec<&String> = result.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["a", "m", "z"]);
    }

    #[test]
    fn canonicalize_is_recursive() {
        let input = serde_json::json!({"outer": {"z": 1, "a": 2}});
        let result = canonicalize_json_value(input);
        let inner_keys: Vec<&String> = result["outer"].as_object().unwrap().keys().collect();
        assert_eq!(inner_keys, vec!["a", "z"]);
    }

    #[test]
    fn canonicalize_preserves_array_order() {
        let input = serde_json::json!([3, 1, 2]);
        let result = canonicalize_json_value(input);
        assert_eq!(result, serde_json::json!([3, 1, 2]));
    }

    #[test]
    fn canonicalize_preserves_scalars() {
        assert_eq!(
            canonicalize_json_value(serde_json::json!("hello")),
            serde_json::json!("hello")
        );
        assert_eq!(
            canonicalize_json_value(serde_json::json!(42)),
            serde_json::json!(42)
        );
        assert_eq!(
            canonicalize_json_value(serde_json::json!(null)),
            serde_json::json!(null)
        );
    }

    // ── floor_char_boundary ──

    #[test]
    fn floor_char_boundary_at_valid_boundary() {
        assert_eq!(floor_char_boundary("hello", 3), 3);
    }

    #[test]
    fn floor_char_boundary_mid_multibyte_char() {
        // h = 1 byte, é = 2 bytes, total 3 bytes
        let s = "hé";
        assert_eq!(floor_char_boundary(s, 2), 1); // byte 2 is mid-é, back up to 1
    }

    #[test]
    fn floor_char_boundary_past_end() {
        assert_eq!(floor_char_boundary("hi", 100), 2);
    }

    #[test]
    fn floor_char_boundary_at_zero() {
        assert_eq!(floor_char_boundary("hello", 0), 0);
    }

    #[test]
    fn floor_char_boundary_empty_string() {
        assert_eq!(floor_char_boundary("", 5), 0);
    }

    // ── ensure_ends_with_user_message ──

    #[test]
    fn ensure_user_message_injects_when_empty() {
        let mut msgs: Vec<ChatMessage> = vec![];
        ensure_ends_with_user_message(&mut msgs);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, crate::llm::Role::User);
    }

    #[test]
    fn ensure_user_message_injects_after_assistant() {
        let mut msgs = vec![ChatMessage::user("hi"), ChatMessage::assistant("hello")];
        ensure_ends_with_user_message(&mut msgs);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2].role, crate::llm::Role::User);
    }

    #[test]
    fn ensure_user_message_injects_after_tool_result() {
        let mut msgs = vec![
            ChatMessage::user("run tool"),
            ChatMessage::tool_result("call_1", "my_tool", "result"),
        ];
        ensure_ends_with_user_message(&mut msgs);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2].role, crate::llm::Role::User);
    }

    #[test]
    fn ensure_user_message_no_op_when_already_user() {
        let mut msgs = vec![ChatMessage::user("hello")];
        ensure_ends_with_user_message(&mut msgs);
        assert_eq!(msgs.len(), 1);
    }
}
