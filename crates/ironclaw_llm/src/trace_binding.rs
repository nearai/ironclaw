//! Exact references from recorded tool arguments to earlier tool results.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceResultBinding {
    tool_call_id: String,
    pointer: String,
}

/// A tool result visible to a trace recorder or replay provider.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservedToolResult {
    pub tool_call_id: String,
    pub content: serde_json::Value,
}

/// Failure while resolving an exact result binding.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TraceBindingError {
    #[error("invalid $trace_result: {0}")]
    InvalidMarker(String),
    #[error("trace result has no tool call with id {0:?}")]
    MissingToolCall(String),
    #[error("trace result has multiple tool results with id {0:?}")]
    DuplicateToolCall(String),
    #[error("trace result for tool call {tool_call_id:?} has no JSON Pointer {pointer:?}")]
    MissingPointer {
        tool_call_id: String,
        pointer: String,
    },
}

/// Resolve every exact `$trace_result` marker in a recorded argument.
///
/// Markers use an assistant tool-call ID plus an RFC 6901 JSON Pointer:
/// `{"$trace_result":{"tool_call_id":"call_1","pointer":"/file/id"}}`.
pub fn resolve_trace_result_bindings(
    value: &mut serde_json::Value,
    observed: &[ObservedToolResult],
) -> Result<(), TraceBindingError> {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                resolve_trace_result_bindings(item, observed)?;
            }
        }
        serde_json::Value::Object(map) if map.contains_key("$trace_result") => {
            if map.len() != 1 {
                return Err(TraceBindingError::InvalidMarker(
                    "marker object must contain only $trace_result".to_string(),
                ));
            }
            let marker = map.get("$trace_result").cloned().ok_or_else(|| {
                TraceBindingError::InvalidMarker("missing marker payload".to_string())
            })?;
            let binding: TraceResultBinding = serde_json::from_value(marker).map_err(|error| {
                TraceBindingError::InvalidMarker(format!(
                    "expected non-empty tool_call_id and JSON Pointer: {error}"
                ))
            })?;
            if binding.tool_call_id.is_empty() {
                return Err(TraceBindingError::InvalidMarker(
                    "tool_call_id must be non-empty".to_string(),
                ));
            }
            if !binding.pointer.is_empty() && !binding.pointer.starts_with('/') {
                return Err(TraceBindingError::InvalidMarker(
                    "pointer must be empty or start with '/'".to_string(),
                ));
            }
            let mut matching_results = observed
                .iter()
                .filter(|result| result.tool_call_id == binding.tool_call_id);
            let result = matching_results
                .next()
                .ok_or_else(|| TraceBindingError::MissingToolCall(binding.tool_call_id.clone()))?;
            if matching_results.next().is_some() {
                return Err(TraceBindingError::DuplicateToolCall(binding.tool_call_id));
            }
            let payload = canonical_tool_result_payload(&result.content).ok_or_else(|| {
                TraceBindingError::MissingPointer {
                    tool_call_id: binding.tool_call_id.clone(),
                    pointer: binding.pointer.clone(),
                }
            })?;
            *value = payload.pointer(&binding.pointer).cloned().ok_or(
                TraceBindingError::MissingPointer {
                    tool_call_id: binding.tool_call_id,
                    pointer: binding.pointer,
                },
            )?;
        }
        serde_json::Value::Object(map) => {
            for item in map.values_mut() {
                resolve_trace_result_bindings(item, observed)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Return the provider JSON inside a complete host evidence envelope.
pub(crate) fn canonical_tool_result_payload(
    content: &serde_json::Value,
) -> Option<Cow<'_, serde_json::Value>> {
    let Some(object) = content.as_object() else {
        return Some(Cow::Borrowed(content));
    };
    if !["schema_version", "status", "trust"]
        .iter()
        .all(|key| object.contains_key(*key))
    {
        return Some(Cow::Borrowed(content));
    }
    let detail = object.get("detail").and_then(serde_json::Value::as_object);
    let preview = detail
        .filter(|detail| {
            detail
                .get("next_offset")
                .is_none_or(serde_json::Value::is_null)
                && matches!(
                    (
                        detail.get("byte_len").and_then(serde_json::Value::as_u64),
                        detail
                            .get("total_bytes")
                            .and_then(serde_json::Value::as_u64),
                    ),
                    (Some(byte_len), Some(total_bytes)) if byte_len == total_bytes
                )
        })
        .and_then(|detail| detail.get("preview"))
        .and_then(serde_json::Value::as_str);
    preview
        .and_then(|preview| serde_json::from_str(preview).ok())
        .map(Cow::Owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_exact_call_id_and_json_pointer() {
        let mut arguments = serde_json::json!({
            "file_id": {
                "$trace_result": {
                    "tool_call_id": "call_upload",
                    "pointer": "/files/0/a~1b~0c"
                }
            }
        });
        let observed = vec![
            ObservedToolResult {
                tool_call_id: "call_upload".to_string(),
                content: serde_json::json!({"files": [{"a/b~c": "fresh"}]}),
            },
            ObservedToolResult {
                tool_call_id: "call_similar".to_string(),
                content: serde_json::json!({"files": [{"a/b~c": "wrong"}]}),
            },
        ];

        resolve_trace_result_bindings(&mut arguments, &observed).expect("binding should resolve");

        assert_eq!(arguments, serde_json::json!({"file_id": "fresh"}));
    }

    #[test]
    fn missing_call_id_does_not_guess() {
        let mut arguments = serde_json::json!({
            "$trace_result": {
                "tool_call_id": "missing",
                "pointer": "/file/id"
            }
        });
        let observed = vec![ObservedToolResult {
            tool_call_id: "call_upload".to_string(),
            content: serde_json::json!({"file": {"id": "must-not-be-used"}}),
        }];

        assert_eq!(
            resolve_trace_result_bindings(&mut arguments, &observed),
            Err(TraceBindingError::MissingToolCall("missing".to_string()))
        );
    }

    #[test]
    fn duplicate_call_id_does_not_guess() {
        let mut arguments = serde_json::json!({
            "$trace_result": {
                "tool_call_id": "call_upload",
                "pointer": "/file/id"
            }
        });
        let observed = vec![
            ObservedToolResult {
                tool_call_id: "call_upload".to_string(),
                content: serde_json::json!({"file": {"id": "first"}}),
            },
            ObservedToolResult {
                tool_call_id: "call_upload".to_string(),
                content: serde_json::json!({"file": {"id": "second"}}),
            },
        ];

        assert_eq!(
            resolve_trace_result_bindings(&mut arguments, &observed),
            Err(TraceBindingError::DuplicateToolCall(
                "call_upload".to_string()
            ))
        );
    }

    #[test]
    fn unwraps_only_complete_host_evidence_previews() {
        let mut complete = serde_json::json!({
            "$trace_result": {
                "tool_call_id": "call_upload",
                "pointer": "/file/id"
            }
        });
        let complete_result = ObservedToolResult {
            tool_call_id: "call_upload".to_string(),
            content: serde_json::json!({
                "schema_version": 1,
                "status": "success",
                "trust": "provider",
                "detail": {
                    "byte_len": 24,
                    "total_bytes": 24,
                    "preview": "{\"file\":{\"id\":\"fresh\"}}"
                }
            }),
        };

        resolve_trace_result_bindings(&mut complete, std::slice::from_ref(&complete_result))
            .expect("complete preview should resolve");
        assert_eq!(complete, serde_json::json!("fresh"));

        let mut truncated_result = complete_result.clone();
        truncated_result.content["detail"]["byte_len"] = serde_json::json!(12);
        let mut truncated = serde_json::json!({
            "$trace_result": {
                "tool_call_id": "call_upload",
                "pointer": "/file/id"
            }
        });
        assert!(matches!(
            resolve_trace_result_bindings(&mut truncated, &[truncated_result]),
            Err(TraceBindingError::MissingPointer { .. })
        ));

        let mut paged = truncated;
        paged["$trace_result"]["pointer"] = serde_json::json!("/detail/preview");
        let mut paged_result = complete_result;
        paged_result.content["detail"]["next_offset"] = serde_json::json!(24);
        assert!(matches!(
            resolve_trace_result_bindings(&mut paged, &[paged_result]),
            Err(TraceBindingError::MissingPointer { .. })
        ));
    }
}
