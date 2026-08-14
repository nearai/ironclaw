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

/// Property tests for the fixture-parsing boundary (#6524 workstream 9).
///
/// Recorded traces are walked here before replay. The content originates from
/// a model and a provider, so the resolver has to stay well-behaved on shapes
/// nobody wrote by hand: markers in odd positions, values that merely look
/// like markers, and structures deeper than any fixture an author would type.
#[cfg(test)]
mod trace_binding_properties {
    use super::*;
    use proptest::prelude::*;

    /// serde_json refuses to parse deeper than this, so no fixture read from
    /// disk can hand the resolver anything more nested. Verified rather than
    /// assumed: depth 127 parses, 128 is rejected with "recursion limit
    /// exceeded". The resolver's own recursion overflows the stack somewhere
    /// between 100 and 1000, so this ceiling — enforced in a different crate —
    /// is the only reason that is unreachable. If serde_json's limit ever
    /// rises, or a caller builds a Value programmatically instead of parsing
    /// one, this function needs an explicit depth guard.
    const SERDE_JSON_PARSE_DEPTH_LIMIT: usize = 128;

    /// Arbitrary JSON, shallow enough to mirror what a parsed fixture can hold.
    fn json_value() -> impl Strategy<Value = serde_json::Value> {
        let leaf = prop_oneof![
            Just(serde_json::Value::Null),
            any::<bool>().prop_map(serde_json::Value::from),
            any::<i64>().prop_map(serde_json::Value::from),
            "\\PC{0,12}".prop_map(serde_json::Value::from),
        ];
        leaf.prop_recursive(6, 48, 4, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..4).prop_map(serde_json::Value::Array),
                proptest::collection::hash_map("[a-z$_]{1,6}", inner, 0..4)
                    .prop_map(|m| serde_json::Value::Object(m.into_iter().collect())),
            ]
        })
    }

    proptest! {
        /// A document with no markers must come back byte-identical.
        ///
        /// The resolver rewrites in place, so a bug here would silently alter
        /// replayed arguments — the trace would still run and would still
        /// look like it reproduced the recording.
        #[test]
        fn documents_without_markers_are_unchanged(value in json_value()) {
            prop_assume!(!format!("{value}").contains("$trace_result"));
            let mut resolved = value.clone();
            let outcome = resolve_trace_result_bindings(&mut resolved, &[]);
            prop_assert!(outcome.is_ok(), "{outcome:?}");
            prop_assert_eq!(resolved, value);
        }

        /// Arbitrary content resolves or errors, never panics.
        #[test]
        fn arbitrary_documents_never_panic(
            value in json_value(),
            ids in proptest::collection::vec("[a-z_0-9]{1,8}", 0..3),
        ) {
            let observed: Vec<_> = ids
                .into_iter()
                .map(|id| ObservedToolResult {
                    tool_call_id: id,
                    content: serde_json::json!({"ok": true}),
                })
                .collect();
            let mut resolved = value;
            let _ = resolve_trace_result_bindings(&mut resolved, &observed);
        }

        /// A well-formed marker is replaced by exactly the pointed-at value.
        #[test]
        fn a_valid_marker_resolves_to_the_pointed_value(
            tool_call_id in "[a-z_0-9]{1,8}",
            payload in json_value(),
        ) {
            let observed = ObservedToolResult {
                tool_call_id: tool_call_id.clone(),
                content: serde_json::json!({"nested": {"value": payload.clone()}}),
            };
            let mut arguments = serde_json::json!({
                "arg": {"$trace_result": {"tool_call_id": tool_call_id, "pointer": "/nested/value"}}
            });
            resolve_trace_result_bindings(&mut arguments, std::slice::from_ref(&observed))
                .expect("a well-formed marker resolves");
            prop_assert_eq!(&arguments["arg"], &payload);
        }

        /// An unknown tool-call id is a clean error, never a silent pass.
        ///
        /// Silently leaving the marker in place would replay a literal
        /// `{"$trace_result": ...}` object as a tool argument, which a
        /// provider would reject far from the real cause.
        #[test]
        fn an_unknown_tool_call_id_errors(
            wanted in "[a-z]{4,8}",
            present in "[A-Z]{4,8}",
        ) {
            let observed = ObservedToolResult {
                tool_call_id: present,
                content: serde_json::json!({"id": 1}),
            };
            let mut arguments = serde_json::json!({
                "arg": {"$trace_result": {"tool_call_id": wanted, "pointer": "/id"}}
            });
            let outcome =
                resolve_trace_result_bindings(&mut arguments, std::slice::from_ref(&observed));
            prop_assert!(matches!(outcome, Err(TraceBindingError::MissingToolCall(_))), "{outcome:?}");
        }
    }

    /// The depth a parsed fixture can actually reach is handled without panic.
    #[test]
    fn nesting_up_to_the_parser_limit_is_handled() {
        let depth = SERDE_JSON_PARSE_DEPTH_LIMIT - 1;
        let text = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
        let mut value: serde_json::Value =
            serde_json::from_str(&text).expect("one under the limit still parses");
        resolve_trace_result_bindings(&mut value, &[]).expect("no markers, so nothing to resolve");
    }

    /// Pins the assumption the comment above rests on: the ceiling is real.
    #[test]
    fn the_parser_rejects_deeper_documents_than_the_resolver_can_walk() {
        let text = format!(
            "{}1{}",
            "[".repeat(SERDE_JSON_PARSE_DEPTH_LIMIT),
            "]".repeat(SERDE_JSON_PARSE_DEPTH_LIMIT)
        );
        assert!(
            serde_json::from_str::<serde_json::Value>(&text).is_err(),
            "serde_json accepted a document at the recursion limit; the \
             resolver's lack of a depth guard is no longer covered by it"
        );
    }
}
