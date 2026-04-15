//! Shared utility functions for the web gateway.

use crate::channels::web::types::{ToolCallInfo, TurnInfo};

pub use ironclaw_common::truncate_preview;

/// Convert stored tool errors into plain text suitable for UI display.
pub fn tool_error_for_display(error: &str) -> String {
    ironclaw_safety::SafetyLayer::unwrap_tool_output(error).unwrap_or_else(|| error.to_string())
}

fn format_turn_error_response(error: &str) -> String {
    let trimmed = error.trim();
    if trimmed.starts_with("Error:") {
        trimmed.to_string()
    } else {
        format!("Error: {trimmed}")
    }
}

/// Parse tool call summary JSON objects into `ToolCallInfo` structs.
fn parse_tool_call_infos(calls: &[serde_json::Value]) -> Vec<ToolCallInfo> {
    calls
        .iter()
        .map(|c| ToolCallInfo {
            name: c["name"].as_str().unwrap_or("unknown").to_string(),
            has_result: c.get("result_preview").is_some_and(|v| !v.is_null()),
            has_error: c.get("error").is_some_and(|v| !v.is_null()),
            result_preview: c["result_preview"].as_str().map(String::from),
            error: c["error"].as_str().map(tool_error_for_display),
            rationale: c["rationale"].as_str().map(String::from),
        })
        .collect()
}

/// Convert an in-memory runtime turn into the history DTO returned by the
/// web gateway. Failed turns synthesize a textual assistant response from the
/// stored error so refreshes preserve the exact failure the live UI surfaced.
pub fn runtime_turn_to_turn_info(turn: &crate::agent::session::Turn) -> TurnInfo {
    TurnInfo {
        turn_number: turn.turn_number,
        user_input: turn.user_input.clone(),
        response: turn
            .response
            .clone()
            .or_else(|| turn.error.as_deref().map(format_turn_error_response)),
        state: format!("{:?}", turn.state),
        started_at: turn.started_at.to_rfc3339(),
        completed_at: turn.completed_at.map(|dt| dt.to_rfc3339()),
        tool_calls: turn
            .tool_calls
            .iter()
            .map(|tc| ToolCallInfo {
                name: tc.name.clone(),
                has_result: tc.result.is_some(),
                has_error: tc.error.is_some(),
                result_preview: tc.result.as_ref().map(|r| {
                    let s = match r {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    truncate_preview(&s, 500)
                }),
                error: tc.error.as_deref().map(tool_error_for_display),
                rationale: tc.rationale.clone(),
            })
            .collect(),
        narrative: turn.narrative.clone(),
    }
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
            // Skip empty standalone assistant messages — these are DB artifacts
            // from code paths that bypassed persist_assistant_response's guard.
            if msg.content.trim().is_empty() {
                continue;
            }
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
    fn test_build_turns_skips_empty_standalone_assistant_messages() {
        // Regression: empty assistant messages in DB (from bypassed persist guard)
        // should not produce ghost turns in the history.
        let messages = vec![
            make_msg("assistant", "", 0),
            make_msg("assistant", "   ", 1000),
            make_msg("assistant", "Real output", 2000),
        ];
        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 1, "empty assistant messages must be filtered");
        assert_eq!(turns[0].response.as_deref(), Some("Real output"));
        assert_eq!(turns[0].turn_number, 0);
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

    #[test]
    fn test_runtime_turn_to_turn_info_synthesizes_failed_response() {
        let mut turn = crate::agent::session::Turn::new(0, "hello");
        turn.fail("LLM error: model_not_found");

        let info = runtime_turn_to_turn_info(&turn);

        assert_eq!(info.state, "Failed");
        assert_eq!(
            info.response.as_deref(),
            Some("Error: LLM error: model_not_found")
        );
    }
}
