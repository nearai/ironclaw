//! Shared utility functions for the web gateway.

use crate::channels::IncomingMessage;
use crate::channels::web::types::{GeneratedImageInfo, ToolCallInfo, TurnInfo};
use crate::generated_images::GeneratedImageSentinel;

pub use ironclaw_common::truncate_preview;

const MAX_HISTORY_IMAGE_DATA_URL_BYTES_PER_IMAGE: usize = 512 * 1024;
const MAX_HISTORY_IMAGE_DATA_URL_BYTES_PER_RESPONSE: usize = 1024 * 1024;

/// Build an incoming message with the metadata invariants expected by the web
/// gateway and downstream status routing.
///
/// Every browser-originated or browser-injected message must carry `user_id`
/// in metadata so `GatewayChannel::send_status()` can scope SSE/WS events to
/// the authenticated user. When a thread is known, mirror it into metadata so
/// downstream status broadcasts and history rehydration stay thread-scoped.
pub fn web_incoming_message_with_metadata(
    channel: impl Into<String>,
    user_id: &str,
    content: impl Into<String>,
    thread_id: Option<&str>,
    metadata: serde_json::Value,
) -> IncomingMessage {
    let mut message = IncomingMessage::new(channel, user_id, content);
    if let Some(thread_id) = thread_id {
        message = message.with_thread(thread_id.to_string());
    }

    let mut metadata = match metadata {
        serde_json::Value::Object(map) => serde_json::Value::Object(map),
        _ => serde_json::json!({}),
    };
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert("user_id".to_string(), serde_json::json!(user_id));
        if let Some(thread_id) = message.thread_id.as_deref() {
            obj.insert("thread_id".to_string(), serde_json::json!(thread_id));
        }
    }

    message.with_metadata(metadata)
}

pub fn web_incoming_message(
    channel: impl Into<String>,
    user_id: &str,
    content: impl Into<String>,
    thread_id: Option<&str>,
) -> IncomingMessage {
    web_incoming_message_with_metadata(channel, user_id, content, thread_id, serde_json::json!({}))
}

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

fn generated_image_event_id(
    turn_number: usize,
    result_index: usize,
    preferred_id: Option<&str>,
) -> String {
    preferred_id
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("turn-{turn_number}-image-{result_index}"))
}

fn parse_image_generated_sentinel_from_value(
    value: &serde_json::Value,
    event_id: String,
) -> Option<GeneratedImageInfo> {
    let sentinel = GeneratedImageSentinel::from_value(value)?;
    let data_url = sentinel
        .data_url()
        .filter(|data_url| !data_url.is_empty())
        .map(str::to_string);
    let path = sentinel.path().map(String::from);
    Some(GeneratedImageInfo {
        event_id,
        data_url,
        path,
    })
}

pub fn collect_generated_images_from_tool_results<'a>(
    turn_number: usize,
    tool_results: impl IntoIterator<Item = (Option<&'a str>, Option<&'a serde_json::Value>)>,
) -> Vec<GeneratedImageInfo> {
    tool_results
        .into_iter()
        .enumerate()
        .filter_map(|(result_index, (event_id, result))| {
            parse_image_generated_sentinel_from_value(
                result?,
                generated_image_event_id(turn_number, result_index, event_id),
            )
        })
        .collect()
}

pub fn tool_result_preview(result: Option<&serde_json::Value>) -> Option<String> {
    let result = result?;
    if GeneratedImageSentinel::from_value(result).is_some() {
        return Some("Generated image".to_string());
    }
    let s = match result {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    Some(tool_result_for_display(&s))
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
                user_message_id: Some(msg.id),
                user_input: msg.content.clone(),
                response: None,
                state: "Completed".to_string(),
                started_at: msg.created_at.to_rfc3339(),
                completed_at: None,
                tool_calls: Vec::new(),
                generated_images: Vec::new(),
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
                        turn.generated_images = collect_generated_images_from_tool_results(
                            turn_number,
                            calls.iter().map(|call| {
                                (
                                    call.get("tool_call_id")
                                        .or_else(|| call.get("call_id"))
                                        .and_then(|v| v.as_str()),
                                    call.get("result"),
                                )
                            }),
                        );
                    }
                    Ok(serde_json::Value::Object(obj)) => {
                        // New wrapped format with narrative
                        turn.narrative = obj
                            .get("narrative")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        if let Some(serde_json::Value::Array(calls)) = obj.get("calls") {
                            turn.tool_calls = parse_tool_call_infos(calls);
                            turn.generated_images = collect_generated_images_from_tool_results(
                                turn_number,
                                calls.iter().map(|call| {
                                    (
                                        call.get("tool_call_id")
                                            .or_else(|| call.get("call_id"))
                                            .and_then(|v| v.as_str()),
                                        call.get("result"),
                                    )
                                }),
                            );
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
                user_message_id: None,
                user_input: String::new(),
                response: Some(msg.content.clone()),
                state: "Completed".to_string(),
                started_at: msg.created_at.to_rfc3339(),
                completed_at: Some(msg.created_at.to_rfc3339()),
                tool_calls: Vec::new(),
                generated_images: Vec::new(),
                narrative: None,
            });
            turn_number += 1;
        }
    }

    turns
}

pub fn enforce_generated_image_history_budget(turns: &mut [TurnInfo]) {
    let mut remaining_bytes = MAX_HISTORY_IMAGE_DATA_URL_BYTES_PER_RESPONSE;
    for turn in turns.iter_mut().rev() {
        for image in turn.generated_images.iter_mut().rev() {
            let Some(data_url) = image.data_url.as_ref() else {
                continue;
            };
            let data_url_bytes = data_url.len();
            if data_url_bytes > MAX_HISTORY_IMAGE_DATA_URL_BYTES_PER_IMAGE
                || data_url_bytes > remaining_bytes
            {
                image.data_url = None;
                continue;
            }
            remaining_bytes -= data_url_bytes;
        }
    }
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
    fn test_tool_result_preview_unwraps_wrapped_content() {
        let wrapped = serde_json::json!(
            "<tool_output name=\"http\">\n{\"city\":\"Shanghai\"}\n</tool_output>"
        );
        assert_eq!(
            tool_result_preview(Some(&wrapped)).as_deref(),
            Some("{\"city\":\"Shanghai\"}")
        );
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

    #[test]
    fn test_collect_generated_images_from_tool_results_parses_stringified_sentinel() {
        let sentinel = serde_json::json!({
            "type": "image_generated",
            "data": "data:image/jpeg;base64,abc123",
            "path": "/tmp/cat.jpg"
        })
        .to_string();
        let tool_results = [serde_json::Value::String(sentinel)];

        let images = collect_generated_images_from_tool_results(
            7,
            tool_results
                .iter()
                .map(|result| (Some("call_img_1"), Some(result))),
        );

        assert_eq!(images.len(), 1);
        assert_eq!(images[0].event_id, "call_img_1");
        assert_eq!(
            images[0].data_url.as_deref(),
            Some("data:image/jpeg;base64,abc123")
        );
        assert_eq!(images[0].path.as_deref(), Some("/tmp/cat.jpg"));
    }

    #[test]
    fn test_build_turns_collects_generated_images_from_persisted_tool_results() {
        let tool_calls = serde_json::json!({
            "calls": [{
                "name": "image_generate",
                "result_preview": "Generated image",
                "result": serde_json::json!({
                    "type": "image_generated",
                    "data": "data:image/jpeg;base64,abc123",
                    "media_type": "image/jpeg"
                }).to_string()
            }]
        });
        let messages = vec![
            make_msg("user", "Draw a cat", 0),
            make_msg("tool_calls", &tool_calls.to_string(), 500),
            make_msg("assistant", "Generated image.", 1000),
        ];

        let turns = build_turns_from_db_messages(&messages);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].generated_images.len(), 1);
        assert_eq!(
            turns[0].generated_images[0].data_url.as_deref(),
            Some("data:image/jpeg;base64,abc123")
        );
    }

    #[test]
    fn test_collect_generated_images_from_double_stringified_sentinel() {
        let sentinel = serde_json::json!({
            "type": "image_generated",
            "data": "data:image/jpeg;base64,abc123",
            "media_type": "image/jpeg"
        })
        .to_string();
        let double_wrapped = serde_json::Value::String(serde_json::to_string(&sentinel).unwrap());

        let images = collect_generated_images_from_tool_results(
            3,
            [(Some("call_img_2"), Some(&double_wrapped))],
        );

        assert_eq!(images.len(), 1);
        assert_eq!(
            images[0].data_url.as_deref(),
            Some("data:image/jpeg;base64,abc123")
        );
    }

    #[test]
    fn test_collect_generated_images_from_data_omitted_sentinel_keeps_placeholder_event() {
        let sentinel = serde_json::json!({
            "type": "image_generated",
            "media_type": "image/png",
            "path": "/tmp/cat.png",
            "data_omitted": true,
            "omitted_reason": "exceeded the 512 KiB cap"
        });

        let images =
            collect_generated_images_from_tool_results(4, [(Some("call_img_3"), Some(&sentinel))]);

        assert_eq!(images.len(), 1);
        assert_eq!(images[0].event_id, "call_img_3");
        assert!(images[0].data_url.is_none());
        assert_eq!(images[0].path.as_deref(), Some("/tmp/cat.png"));
    }

    #[test]
    fn test_build_turns_assign_distinct_event_ids_for_identical_generated_images() {
        let shared_sentinel = serde_json::json!({
            "type": "image_generated",
            "data": "data:image/png;base64,shared",
            "media_type": "image/png"
        })
        .to_string();
        let turn_one_calls = serde_json::json!({
            "calls": [{
                "name": "image_generate",
                "tool_call_id": "call_turn_1",
                "result_preview": "Generated image",
                "result": shared_sentinel
            }]
        });
        let turn_two_calls = serde_json::json!({
            "calls": [{
                "name": "image_generate",
                "tool_call_id": "call_turn_2",
                "result_preview": "Generated image",
                "result": serde_json::json!({
                    "type": "image_generated",
                    "data": "data:image/png;base64,shared",
                    "media_type": "image/png"
                }).to_string()
            }]
        });
        let messages = vec![
            make_msg("user", "Draw one", 0),
            make_msg("tool_calls", &turn_one_calls.to_string(), 500),
            make_msg("assistant", "Done", 1000),
            make_msg("user", "Draw it again", 2000),
            make_msg("tool_calls", &turn_two_calls.to_string(), 2500),
            make_msg("assistant", "Done again", 3000),
        ];

        let turns = build_turns_from_db_messages(&messages);

        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].generated_images[0].event_id, "call_turn_1");
        assert_eq!(turns[1].generated_images[0].event_id, "call_turn_2");
        assert_ne!(
            turns[0].generated_images[0].event_id,
            turns[1].generated_images[0].event_id
        );
    }

    #[test]
    fn test_enforce_generated_image_history_budget_caps_total_bytes() {
        let oversized_data_url = format!(
            "data:image/png;base64,{}",
            "a".repeat(MAX_HISTORY_IMAGE_DATA_URL_BYTES_PER_IMAGE - 4096)
        );
        let mut turns = vec![
            TurnInfo {
                turn_number: 0,
                user_message_id: None,
                user_input: "older".to_string(),
                response: Some("done".to_string()),
                state: "Completed".to_string(),
                started_at: chrono::Utc::now().to_rfc3339(),
                completed_at: Some(chrono::Utc::now().to_rfc3339()),
                tool_calls: Vec::new(),
                generated_images: vec![GeneratedImageInfo {
                    event_id: "old".to_string(),
                    data_url: Some(oversized_data_url.clone()),
                    path: None,
                }],
                narrative: None,
            },
            TurnInfo {
                turn_number: 1,
                user_message_id: None,
                user_input: "newer".to_string(),
                response: Some("done".to_string()),
                state: "Completed".to_string(),
                started_at: chrono::Utc::now().to_rfc3339(),
                completed_at: Some(chrono::Utc::now().to_rfc3339()),
                tool_calls: Vec::new(),
                generated_images: vec![
                    GeneratedImageInfo {
                        event_id: "new-1".to_string(),
                        data_url: Some(oversized_data_url.clone()),
                        path: None,
                    },
                    GeneratedImageInfo {
                        event_id: "new-2".to_string(),
                        data_url: Some(oversized_data_url.clone()),
                        path: None,
                    },
                ],
                narrative: None,
            },
        ];

        enforce_generated_image_history_budget(&mut turns);

        let total_bytes: usize = turns
            .iter()
            .flat_map(|turn| turn.generated_images.iter())
            .filter_map(|image| image.data_url.as_ref())
            .map(|data_url| data_url.len())
            .sum();

        assert!(total_bytes <= MAX_HISTORY_IMAGE_DATA_URL_BYTES_PER_RESPONSE);
        assert!(turns[0].generated_images[0].data_url.is_none());
        assert!(turns[1].generated_images[0].data_url.is_some());
        assert!(turns[1].generated_images[1].data_url.is_some());
    }
}
