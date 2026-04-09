//! Shared utility functions for the web gateway.

use crate::channels::web::types::{ToolCallInfo, TurnInfo};

pub use ironclaw_common::truncate_preview;

/// Convert stored tool errors into plain text suitable for UI display.
pub fn tool_error_for_display(error: &str) -> String {
    ironclaw_safety::SafetyLayer::unwrap_tool_output(error).unwrap_or_else(|| error.to_string())
}

/// Convert stored tool result content into plain text suitable for UI display.
pub fn tool_result_for_display(content: &str) -> String {
    let unwrapped = ironclaw_safety::SafetyLayer::unwrap_tool_output(content)
        .unwrap_or_else(|| content.to_string());
    truncate_preview(&unwrapped, 1000)
}

/// Parse tool call summary JSON objects into `ToolCallInfo` structs.
fn parse_tool_call_infos(calls: &[serde_json::Value]) -> Vec<ToolCallInfo> {
    calls
        .iter()
        .map(|c| {
            let result_source = c
                .get("result")
                .or_else(|| c.get("result_preview"))
                .and_then(|v| v.as_str());
            ToolCallInfo {
                name: c["name"].as_str().unwrap_or("unknown").to_string(),
                has_result: c
                    .get("result")
                    .or_else(|| c.get("result_preview"))
                    .is_some_and(|v| !v.is_null()),
                has_error: c.get("error").is_some_and(|v| !v.is_null()),
                result_preview: result_source.map(tool_result_for_display),
                error: c["error"].as_str().map(tool_error_for_display),
                rationale: c["rationale"].as_str().map(String::from),
            }
        })
        .collect()
}

/// Build TurnInfo pairs from flat DB messages (user/tool_calls/assistant triples).
///
/// Handles three message patterns:
/// - `user → assistant` (legacy, no tool calls)
/// - `user → tool_calls → assistant` (with persisted tool call summaries)
/// - `user` alone (incomplete turn)
pub fn build_turns_from_db_messages(
    messages: &[crate::history::ConversationMessage],
) -> Vec<TurnInfo> {
    let mut turns = Vec::new();
    let mut turn_number = 0;
    let mut iter = messages.iter().peekable();

    while let Some(msg) = iter.next() {
        if msg.role == "user" {
            let mut turn = TurnInfo {
                turn_number,
                user_input: msg.content.clone(),
                response: None,
                state: "Completed".to_string(),
                started_at: msg.created_at.to_rfc3339(),
                completed_at: None,
                tool_calls: Vec::new(),
                narrative: None,
            };

            // Check if next message is a tool_calls record
            if let Some(next) = iter.peek()
                && next.role == "tool_calls"
            {
                let tc_msg = iter.next().expect("peeked");
                // Parse tool_calls JSON — supports two formats:
                // safety: no byte-index slicing; comment describes JSON shape
                match serde_json::from_str::<serde_json::Value>(&tc_msg.content) {
                    Ok(serde_json::Value::Array(calls)) => {
                        // Old format: plain array
                        turn.tool_calls = parse_tool_call_infos(&calls);
                    }
                    Ok(serde_json::Value::Object(obj)) => {
                        // New wrapped format with narrative
                        turn.narrative = obj
                            .get("narrative")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        if let Some(serde_json::Value::Array(calls)) = obj.get("calls") {
                            turn.tool_calls = parse_tool_call_infos(calls);
                        }
                    }
                    Ok(_) => {
                        tracing::warn!(
                            message_id = %tc_msg.id,
                            "Unexpected tool_calls JSON shape in DB, skipping"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            message_id = %tc_msg.id,
                            "Malformed tool_calls JSON in DB, skipping: {e}"
                        );
                    }
                }
            }

            // Check if next message is an assistant response
            if let Some(next) = iter.peek()
                && next.role == "assistant"
            {
                let assistant_msg = iter.next().expect("peeked");
                turn.response = Some(assistant_msg.content.clone());
                turn.completed_at = Some(assistant_msg.created_at.to_rfc3339());
            }

            // Incomplete turn (user message without response)
            if turn.response.is_none() {
                turn.state = "Failed".to_string();
            }

            turns.push(turn);
            turn_number += 1;
        } else if msg.role == "assistant" {
            // Standalone assistant message (e.g. routine output, heartbeat)
            // with no preceding user message — render as a turn with empty input.
            turns.push(TurnInfo {
                turn_number,
                user_input: String::new(),
                response: Some(msg.content.clone()),
                state: "Completed".to_string(),
                started_at: msg.created_at.to_rfc3339(),
                completed_at: Some(msg.created_at.to_rfc3339()),
                tool_calls: Vec::new(),
                narrative: None,
            });
            turn_number += 1;
        }
    }

    turns
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    // ---- build_turns_from_db_messages tests ----

    fn make_msg(role: &str, content: &str, offset_ms: i64) -> crate::history::ConversationMessage {
        crate::history::ConversationMessage {
            id: Uuid::new_v4(),
            role: role.to_string(),
            content: content.to_string(),
            created_at: chrono::Utc::now() + chrono::TimeDelta::milliseconds(offset_ms),
        }
    }

    #[test]
    fn test_build_turns_complete() {
        let messages = vec![
            make_msg("user", "Hello", 0),
            make_msg("assistant", "Hi!", 1000),
            make_msg("user", "How?", 2000),
            make_msg("assistant", "Good", 3000),
        ];
        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user_input, "Hello");
        assert_eq!(turns[0].response.as_deref(), Some("Hi!"));
        assert_eq!(turns[0].state, "Completed");
        assert_eq!(turns[1].user_input, "How?");
        assert_eq!(turns[1].response.as_deref(), Some("Good"));
    }

    #[test]
    fn test_build_turns_incomplete() {
        let messages = vec![make_msg("user", "Hello", 0)];
        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 1);
        assert!(turns[0].response.is_none());
        assert_eq!(turns[0].state, "Failed");
    }

    #[test]
    fn test_build_turns_with_tool_calls() {
        let tc_json = serde_json::json!([
            {"name": "shell", "result_preview": "output"},
            {"name": "http", "error": "timeout"}
        ]);
        let messages = vec![
            make_msg("user", "Run it", 0),
            make_msg("tool_calls", &tc_json.to_string(), 500),
            make_msg("assistant", "Done", 1000),
        ];
        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].tool_calls.len(), 2);
        assert_eq!(turns[0].tool_calls[0].name, "shell");
        assert!(turns[0].tool_calls[0].has_result);
        assert_eq!(turns[0].tool_calls[1].name, "http");
        assert!(turns[0].tool_calls[1].has_error);
        assert_eq!(turns[0].response.as_deref(), Some("Done"));
    }

    #[test]
    fn test_build_turns_unwrap_wrapped_tool_error_for_display() {
        let tc_json = serde_json::json!([
            {
                "name": "http",
                "error": "<tool_output name=\"http\">\nTool 'http' failed: timeout\n</tool_output>"
            }
        ]);
        let messages = vec![
            make_msg("user", "Run it", 0),
            make_msg("tool_calls", &tc_json.to_string(), 500),
        ];

        let turns = build_turns_from_db_messages(&messages);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].tool_calls.len(), 1);
        assert_eq!(
            turns[0].tool_calls[0].error.as_deref(),
            Some("Tool 'http' failed: timeout")
        );
    }

    #[test]
    fn test_tool_result_for_display_unwraps_wrapped_content() {
        let wrapped = "<tool_output name=\"http\">\n{\"city\":\"Shanghai\"}\n</tool_output>";
        assert_eq!(tool_result_for_display(wrapped), "{\"city\":\"Shanghai\"}");
    }

    #[test]
    fn test_build_turns_prefers_full_result_over_preview() {
        let tc_json = serde_json::json!({
            "calls": [{
                "name": "web_search",
                "result_preview": "short preview...",
                "result": "<tool_output name=\"web_search\">\nfull result body\n</tool_output>"
            }]
        });
        let messages = vec![
            make_msg("user", "Search", 0),
            make_msg("tool_calls", &tc_json.to_string(), 500),
            make_msg("assistant", "Done", 1000),
        ];

        let turns = build_turns_from_db_messages(&messages);

        assert_eq!(
            turns[0].tool_calls[0].result_preview.as_deref(),
            Some("full result body")
        );
    }

    #[test]
    fn test_build_turns_malformed_tool_calls() {
        let messages = vec![
            make_msg("user", "Hello", 0),
            make_msg("tool_calls", "not json", 500),
            make_msg("assistant", "Done", 1000),
        ];
        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 1);
        assert!(turns[0].tool_calls.is_empty());
        assert_eq!(turns[0].response.as_deref(), Some("Done"));
    }

    #[test]
    fn test_build_turns_standalone_assistant_messages() {
        // Routine conversations only have assistant messages (no user messages).
        let messages = vec![
            make_msg("assistant", "Routine executed: all checks passed", 0),
            make_msg("assistant", "Routine executed: found 2 issues", 5000),
        ];
        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 2);
        // Standalone assistant messages should have empty user_input
        assert_eq!(turns[0].user_input, "");
        assert_eq!(
            turns[0].response.as_deref(),
            Some("Routine executed: all checks passed")
        );
        assert_eq!(turns[0].state, "Completed");
        assert_eq!(turns[1].user_input, "");
        assert_eq!(
            turns[1].response.as_deref(),
            Some("Routine executed: found 2 issues")
        );
    }

    #[test]
    fn test_build_turns_backward_compatible() {
        let messages = vec![
            make_msg("user", "Hello", 0),
            make_msg("assistant", "Hi!", 1000),
        ];
        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 1);
        assert!(turns[0].tool_calls.is_empty());
        assert_eq!(turns[0].state, "Completed");
    }

    #[test]
    fn test_build_turns_with_wrapped_tool_calls_format() {
        let tc_json = serde_json::json!({
            "narrative": "Searching memory for context before proceeding.",
            "calls": [
                {"name": "memory_search", "result_preview": "found 3 items", "rationale": "consult prior context"},
                {"name": "shell", "error": "permission denied"}
            ]
        });
        let messages = vec![
            make_msg("user", "Find info", 0),
            make_msg("tool_calls", &tc_json.to_string(), 500),
            make_msg("assistant", "Here's what I found", 1000),
        ];
        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].narrative.as_deref(),
            Some("Searching memory for context before proceeding.")
        );
        assert_eq!(turns[0].tool_calls.len(), 2);
        assert_eq!(turns[0].tool_calls[0].name, "memory_search");
        assert_eq!(
            turns[0].tool_calls[0].rationale.as_deref(),
            Some("consult prior context")
        );
        assert!(turns[0].tool_calls[0].has_result);
        assert_eq!(turns[0].tool_calls[1].name, "shell");
        assert!(turns[0].tool_calls[1].has_error);
        assert_eq!(turns[0].response.as_deref(), Some("Here's what I found"));
    }

    #[test]
    fn test_build_turns_wrapped_format_without_narrative() {
        let tc_json = serde_json::json!({
            "calls": [{"name": "echo", "result_preview": "hello"}]
        });
        let messages = vec![
            make_msg("user", "Say hi", 0),
            make_msg("tool_calls", &tc_json.to_string(), 500),
            make_msg("assistant", "Done", 1000),
        ];
        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 1);
        assert!(turns[0].narrative.is_none());
        assert_eq!(turns[0].tool_calls.len(), 1);
    }
}
