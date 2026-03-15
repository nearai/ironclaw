//! A2A (Agent-to-Agent) protocol parsing: SSE stream events and JSON-RPC 2.0.
//!
//! This module is intentionally generic — it handles only protocol-level
//! concerns (SSE framing, JSON-RPC envelope, event classification) and can
//! be reused by any A2A integration.

/// A parsed SSE event from an A2A stream.
#[derive(Debug, Clone)]
pub(crate) struct A2aStreamEvent {
    /// The SSE `event:` field (e.g. `"message"`). `None` for unnamed events.
    /// Retained for future event-type filtering (e.g. skipping non-message events).
    #[allow(dead_code)]
    pub event_type: Option<String>,
    /// Parsed JSON from the `data:` field.
    pub raw: serde_json::Value,
}

/// Classification of an A2A stream event.
#[derive(Debug)]
pub(crate) enum EventKind {
    /// JSON-RPC-level error (top-level `error` field) or result-level error.
    Error(String),
    /// Final result available (task completed synchronously or stream finished).
    Final(serde_json::Value),
    /// Task is in progress; contains the task ID and optional context ID.
    InProgress {
        task_id: String,
        context_id: Option<String>,
    },
}

/// Build an A2A JSON-RPC 2.0 request body for `message/stream`.
pub(crate) fn build_jsonrpc_request(
    query: &str,
    context: Option<&serde_json::Value>,
    thread_id: Option<&str>,
) -> serde_json::Value {
    let mut parts = vec![serde_json::json!({ "kind": "text", "text": query })];

    if let Some(ctx) = context {
        parts.push(serde_json::json!({ "kind": "data", "data": ctx }));
    }

    let msg_id = format!("msg-{}", uuid::Uuid::new_v4());
    let mut params = serde_json::json!({
        "message": {
            "role": "user",
            "parts": parts,
            "messageId": msg_id,
        }
    });

    if let Some(tid) = thread_id {
        params["thread"] = serde_json::json!({ "threadId": tid });
    }

    serde_json::json!({
        "jsonrpc": "2.0",
        "id": uuid::Uuid::new_v4().to_string(),
        "method": "message/stream",
        "params": params,
    })
}

/// Parse complete SSE events from a buffer, consuming processed bytes.
///
/// Follows the SSE specification:
/// - Events are delimited by blank lines (`\n\n`)
/// - Multi-line `data:` fields are concatenated with `\n`
/// - `event:` lines set the event type
/// - Lines starting with `:` are comments (ignored)
/// - `\r\n` and `\r` line endings are normalized
pub(crate) fn parse_sse_events(buffer: &mut String) -> Vec<A2aStreamEvent> {
    // Normalize line endings: \r\n → \n, bare \r → \n
    if buffer.contains('\r') {
        *buffer = buffer.replace("\r\n", "\n").replace('\r', "\n");
    }

    let mut events = Vec::new();

    // Process complete event blocks (terminated by \n\n)
    while let Some(boundary) = buffer.find("\n\n") {
        let block = buffer[..boundary].to_string();
        // Remove the block + both newlines from the buffer
        *buffer = buffer[boundary + 2..].to_string();

        if block.is_empty() {
            continue;
        }

        let mut event_type: Option<String> = None;
        let mut data_lines: Vec<&str> = Vec::new();

        for line in block.lines() {
            if line.starts_with(':') {
                // Comment line — skip
                continue;
            }

            if let Some(value) = line.strip_prefix("event:") {
                event_type = Some(value.trim().to_string());
            } else if let Some(value) = line.strip_prefix("data:") {
                let trimmed = value.strip_prefix(' ').unwrap_or(value);
                data_lines.push(trimmed);
            }
            // Ignore `id:`, `retry:`, and unknown fields per SSE spec
        }

        if data_lines.is_empty() {
            continue;
        }

        let data_str = data_lines.join("\n");
        if data_str.is_empty() {
            continue;
        }

        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data_str) {
            events.push(A2aStreamEvent {
                event_type,
                raw: parsed,
            });
        } else {
            tracing::debug!(
                data = %data_str,
                "A2A SSE: skipping non-JSON data block"
            );
        }
    }

    events
}

/// Classify an A2A stream event into an actionable kind.
///
/// Checks for JSON-RPC-level errors first (top-level `error` field), then
/// result-level errors, then final/in-progress status.
pub(crate) fn classify_event(event: &A2aStreamEvent) -> EventKind {
    // C3: Check top-level JSON-RPC error field first
    if let Some(error) = event.raw.get("error") {
        let msg = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown JSON-RPC error");
        let code = error
            .get("code")
            .and_then(|c| c.as_i64())
            .map(|c| format!(" (code: {})", c))
            .unwrap_or_default();
        return EventKind::Error(format!("{}{}", msg, code));
    }

    let result = match event.raw.get("result") {
        Some(r) => r,
        None => return EventKind::Error("A2A event missing 'result' field".to_string()),
    };

    // Check for result-level error
    if result.get("kind").and_then(|k| k.as_str()) == Some("error") {
        let msg = result
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .or_else(|| result.get("message").and_then(|m| m.as_str()))
            .unwrap_or("unknown error");
        return EventKind::Error(msg.to_string());
    }

    // Check if final
    if result.get("final").and_then(|f| f.as_bool()) == Some(true) {
        return EventKind::Final(result.clone());
    }

    // Extract task ID and context ID for in-progress events
    let task_id = result
        .get("id")
        .and_then(|id| id.as_str())
        .unwrap_or("unknown")
        .to_string();

    let context_id = result
        .get("contextId")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string());

    EventKind::InProgress {
        task_id,
        context_id,
    }
}

/// Extract text content from an A2A result's message parts.
///
/// Tries `status.message.parts[].text`, then `message.parts[].text`,
/// then falls back to pretty-printing the entire result.
pub(crate) fn extract_text_from_result(result: &serde_json::Value, max_len: usize) -> String {
    let text = extract_text_parts(result.get("status").and_then(|s| s.get("message")))
        .or_else(|| extract_text_parts(result.get("message")))
        // LangGraph: extract from the last agent message in history[]
        .or_else(|| {
            result
                .get("history")
                .and_then(|h| h.as_array())
                .and_then(|arr| {
                    arr.iter()
                        .rev()
                        .find(|msg| msg.get("role").and_then(|r| r.as_str()) == Some("agent"))
                })
                .and_then(|msg| extract_text_parts(Some(msg)))
        })
        // LangGraph: extract from artifacts[].parts[].text
        .or_else(|| {
            result
                .get("artifacts")
                .and_then(|a| a.as_array())
                .and_then(|arr| arr.first())
                .and_then(|artifact| extract_text_parts(Some(artifact)))
        })
        .unwrap_or_else(|| serde_json::to_string_pretty(result).unwrap_or_default());

    truncate_str(&text, max_len)
}

/// Check if an A2A result has non-empty text parts in `status.message.parts`.
pub(crate) fn result_has_text_parts(result: &serde_json::Value) -> bool {
    result
        .get("status")
        .and_then(|s| s.get("message"))
        .and_then(|m| m.get("parts"))
        .and_then(|p| p.as_array())
        .is_some_and(|parts| {
            parts.iter().any(|part| {
                part.get("text")
                    .and_then(|t| t.as_str())
                    .is_some_and(|s| !s.is_empty())
            })
        })
}

/// Check if a raw event JSON contains meaningful message text at `result.status.message.parts`.
pub(crate) fn has_message_content(raw: &serde_json::Value) -> bool {
    raw.get("result").is_some_and(result_has_text_parts)
}

/// Extract and join text parts from a message object (`{ "parts": [{"text": ...}] }`).
fn extract_text_parts(message: Option<&serde_json::Value>) -> Option<String> {
    message
        .and_then(|m| m.get("parts"))
        .and_then(|p| p.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|s| !s.is_empty())
}

/// Truncate a string to `max_len` bytes, respecting UTF-8 char boundaries.
pub(crate) fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── build_jsonrpc_request ───────────────────────────────────────

    #[test]
    fn build_request_basic() {
        let req = build_jsonrpc_request("hello", None, None);
        assert_eq!(req["method"], "message/stream");
        assert_eq!(req["params"]["message"]["role"], "user");
        let parts = req["params"]["message"]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["text"], "hello");
    }

    #[test]
    fn build_request_with_context_and_thread() {
        let ctx = serde_json::json!({"key": "value"});
        let req = build_jsonrpc_request("query", Some(&ctx), Some("thread-42"));
        let parts = req["params"]["message"]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1]["kind"], "data");
        assert_eq!(req["params"]["thread"]["threadId"], "thread-42");
    }

    // ── parse_sse_events ────────────────────────────────────────────

    #[test]
    fn parse_single_event() {
        let mut buf = "data: {\"result\":{\"id\":\"t1\"}}\n\n".to_string();
        let events = parse_sse_events(&mut buf);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].raw["result"]["id"], "t1");
        assert!(buf.is_empty());
    }

    #[test]
    fn parse_multiple_events() {
        let mut buf = "data: {\"a\":1}\n\ndata: {\"b\":2}\n\ndata: {\"c\":3}\n\n".to_string();
        let events = parse_sse_events(&mut buf);
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn parse_incomplete_event_stays_in_buffer() {
        let mut buf = "data: {\"partial\":true}".to_string();
        let events = parse_sse_events(&mut buf);
        assert!(events.is_empty());
        assert!(!buf.is_empty()); // data remains
    }

    #[test]
    fn parse_multiline_data() {
        let mut buf = "data: {\"multi\":\n\ndata: true}\n\n".to_string();
        // First block: "data: {\"multi\":" — incomplete JSON, will be skipped
        // Second block: "data: true}" — also not valid JSON
        let events = parse_sse_events(&mut buf);
        // Both blocks produce invalid JSON, so no events parsed
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn parse_multiline_data_concatenation() {
        // Two data: lines in the same event block should be concatenated
        let mut buf = "data: {\"key\":\ndata: \"value\"}\n\n".to_string();
        let events = parse_sse_events(&mut buf);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].raw["key"], "value");
    }

    #[test]
    fn parse_event_with_type() {
        let mut buf = "event: message\ndata: {\"ok\":true}\n\n".to_string();
        let events = parse_sse_events(&mut buf);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type.as_deref(), Some("message"));
    }

    #[test]
    fn parse_comment_lines_ignored() {
        let mut buf = ": keep-alive\ndata: {\"ok\":true}\n\n".to_string();
        let events = parse_sse_events(&mut buf);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn parse_crlf_line_endings() {
        let mut buf = "data: {\"ok\":true}\r\n\r\n".to_string();
        let events = parse_sse_events(&mut buf);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn parse_bare_cr_line_endings() {
        let mut buf = "data: {\"ok\":true}\r\r".to_string();
        let events = parse_sse_events(&mut buf);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn parse_empty_data_skipped() {
        let mut buf = "data: \n\n".to_string();
        let events = parse_sse_events(&mut buf);
        assert!(events.is_empty());
    }

    // ── classify_event ──────────────────────────────────────────────

    #[test]
    fn classify_jsonrpc_error() {
        let event = A2aStreamEvent {
            event_type: None,
            raw: serde_json::json!({
                "jsonrpc": "2.0",
                "error": {"code": -32600, "message": "Invalid Request"}
            }),
        };
        match classify_event(&event) {
            EventKind::Error(msg) => assert!(msg.contains("Invalid Request")),
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn classify_result_level_error() {
        let event = A2aStreamEvent {
            event_type: None,
            raw: serde_json::json!({
                "result": {
                    "kind": "error",
                    "error": {"message": "rate limited"}
                }
            }),
        };
        match classify_event(&event) {
            EventKind::Error(msg) => assert_eq!(msg, "rate limited"),
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn classify_final_event() {
        let event = A2aStreamEvent {
            event_type: None,
            raw: serde_json::json!({
                "result": {
                    "final": true,
                    "status": {
                        "state": "completed",
                        "message": {"parts": [{"text": "done"}]}
                    }
                }
            }),
        };
        match classify_event(&event) {
            EventKind::Final(result) => assert_eq!(result["status"]["state"], "completed"),
            _ => panic!("expected Final"),
        }
    }

    #[test]
    fn classify_in_progress_with_context_id() {
        let event = A2aStreamEvent {
            event_type: None,
            raw: serde_json::json!({
                "result": {
                    "id": "task-abc",
                    "contextId": "ctx-123",
                    "final": false
                }
            }),
        };
        match classify_event(&event) {
            EventKind::InProgress {
                task_id,
                context_id,
            } => {
                assert_eq!(task_id, "task-abc");
                assert_eq!(context_id.as_deref(), Some("ctx-123"));
            }
            _ => panic!("expected InProgress"),
        }
    }

    #[test]
    fn classify_missing_result_is_error() {
        let event = A2aStreamEvent {
            event_type: None,
            raw: serde_json::json!({"jsonrpc": "2.0", "id": "1"}),
        };
        match classify_event(&event) {
            EventKind::Error(msg) => assert!(msg.contains("missing 'result'")),
            _ => panic!("expected Error"),
        }
    }

    // ── extract_text_from_result ────────────────────────────────────

    #[test]
    fn extract_text_from_status_message() {
        let result = serde_json::json!({
            "status": {
                "message": {
                    "parts": [
                        {"text": "line one"},
                        {"text": "line two"}
                    ]
                }
            }
        });
        let text = extract_text_from_result(&result, 2000);
        assert_eq!(text, "line one\nline two");
    }

    #[test]
    fn extract_text_truncates_at_char_boundary() {
        let result = serde_json::json!({
            "status": {
                "message": {
                    "parts": [{"text": "你好世界abcdefghij"}]
                }
            }
        });
        // "你好世界" is 12 bytes in UTF-8 (3 bytes each)
        let text = extract_text_from_result(&result, 10);
        assert!(text.ends_with("..."));
        assert!(text.len() <= 13); // 9 (3 chars) + "..."
    }

    #[test]
    fn extract_text_from_langgraph_history() {
        // LangGraph returns agent messages in `history[]`, not `status.message`
        let result = serde_json::json!({
            "id": "task-1",
            "contextId": "ctx-1",
            "history": [
                {"role": "user", "parts": [{"text": "What is 2+2?"}]},
                {"role": "agent", "parts": [{"text": "4"}]}
            ],
            "status": {"state": "completed"},
            "artifacts": [{"parts": [{"text": "4"}]}]
        });
        let text = extract_text_from_result(&result, 2000);
        assert_eq!(text, "4");
    }

    #[test]
    fn extract_text_from_langgraph_artifacts() {
        // When history has no agent messages, fall back to artifacts
        let result = serde_json::json!({
            "artifacts": [
                {"artifactId": "a1", "parts": [{"text": "result content"}]}
            ],
            "status": {"state": "completed"}
        });
        let text = extract_text_from_result(&result, 2000);
        assert_eq!(text, "result content");
    }

    // ── truncate_str ────────────────────────────────────────────────

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let s = "分析茅台的估值";
        let t = truncate_str(s, 6); // 6 bytes = 2 Chinese chars
        assert_eq!(t, "分析...");
    }
}
