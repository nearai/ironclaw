//! Shared utility functions for the web gateway.

use crate::channels::web::types::{ToolCallInfo, TurnInfo};

/// Truncate a string to at most `max_bytes` bytes at a char boundary, appending "...".
pub fn truncate_preview(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    // Walk backwards from max_bytes to find a valid char boundary
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
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
    let mut turn_number = 1;
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
                reasoning: None,
            };

            // Check if next message is a tool_calls record
            if let Some(next) = iter.peek()
                && next.role == "tool_calls"
            {
                let tc_msg = iter.next().expect("peeked");
                match serde_json::from_str::<Vec<serde_json::Value>>(&tc_msg.content) {
                    Ok(calls) => {
                        turn.tool_calls = calls
                            .iter()
                            .map(|c| ToolCallInfo {
                                name: c["name"].as_str().unwrap_or("unknown").to_string(),
                                has_result: c.get("result_preview").is_some(),
                                has_error: c.get("error").is_some(),
                                result_preview: c["result_preview"].as_str().map(String::from),
                                error: c["error"].as_str().map(String::from),
                            })
                            .collect();
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
        }
    }

    turns
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    // ---- truncate_preview tests ----

    #[test]
    fn test_truncate_preview_short_string() {
        assert_eq!(truncate_preview("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_preview_exact_boundary() {
        assert_eq!(truncate_preview("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_preview_truncates_ascii() {
        assert_eq!(truncate_preview("hello world", 5), "hello...");
    }

    #[test]
    fn test_truncate_preview_empty_string() {
        assert_eq!(truncate_preview("", 10), "");
    }

    #[test]
    fn test_truncate_preview_multibyte_char_boundary() {
        // '€' is 3 bytes (E2 82 AC). "a€b" = [61, E2, 82, AC, 62] = 5 bytes
        // Truncating at max_bytes=3 should not split the euro sign.
        let s = "a€b";
        let result = truncate_preview(s, 3);
        // max_bytes=3 lands mid-€, so it walks back to byte 1 ("a")
        assert_eq!(result, "a...");
    }

    #[test]
    fn test_truncate_preview_emoji() {
        // '🦀' is 4 bytes. "hi🦀" = 6 bytes
        let s = "hi🦀";
        let result = truncate_preview(s, 4);
        // max_bytes=4 lands mid-🦀, walks back to byte 2 ("hi")
        assert_eq!(result, "hi...");
    }

    #[test]
    fn test_truncate_preview_cjk() {
        // CJK characters are 3 bytes each. "你好世界" = 12 bytes
        let s = "你好世界";
        let result = truncate_preview(s, 7);
        // max_bytes=7 lands mid-character (byte 7 is inside 世), walks back to 6 ("你好")
        assert_eq!(result, "你好...");
    }

    #[test]
    fn test_truncate_preview_zero_max_bytes() {
        assert_eq!(truncate_preview("hello", 0), "...");
    }

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
}
