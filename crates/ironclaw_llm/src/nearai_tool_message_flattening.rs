use ironclaw_common::provider_transcript::format_tool_result_observation;

use super::{ChatCompletionMessage, MessageContent};

/// Rewrite tool-call / tool-result messages into neutral transcript text.
///
/// NEAR AI cloud-api does not support the OpenAI multi-turn tool-calling
/// protocol (`role: "tool"` messages). This adapter keeps assistant prose but
/// drops assistant tool-call protocol events so the model does not learn an
/// assistant-authored tool-event completion pattern. Tool results become
/// user-side observations.
pub(super) fn flatten_tool_messages(
    messages: Vec<ChatCompletionMessage>,
) -> Vec<ChatCompletionMessage> {
    let has_tool_history = messages
        .iter()
        .any(|m| m.role == "tool" || (m.role == "assistant" && m.tool_calls.is_some()));
    if !has_tool_history {
        return messages;
    }

    tracing::debug!("Flattening tool messages for NEAR AI compatibility");

    messages
        .into_iter()
        .flat_map(flatten_tool_message)
        .collect()
}

fn flatten_tool_message(mut msg: ChatCompletionMessage) -> Vec<ChatCompletionMessage> {
    if msg.role == "assistant" && msg.tool_calls.is_some() {
        msg.tool_call_id = None;
        msg.tool_calls = None;
        if message_has_text(&msg) {
            vec![msg]
        } else {
            Vec::new()
        }
    } else if msg.role == "tool" {
        vec![tool_result_observation(msg)]
    } else {
        vec![msg]
    }
}

fn message_has_text(msg: &ChatCompletionMessage) -> bool {
    msg.content
        .as_ref()
        .and_then(|content| content.as_text())
        .is_some_and(|text| !text.trim().is_empty())
}

fn tool_result_observation(msg: ChatCompletionMessage) -> ChatCompletionMessage {
    let tool_name = msg.name.as_deref().unwrap_or("unknown");
    let result = msg.content.as_ref().and_then(|c| c.as_text()).unwrap_or("");
    ChatCompletionMessage {
        role: "user".to_string(),
        content: Some(MessageContent::Text(format_tool_result_observation(
            tool_name, result,
        ))),
        tool_call_id: None,
        name: None,
        tool_calls: None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ChatCompletionToolCall, ChatCompletionToolCallFunction};
    use super::*;

    #[test]
    fn flatten_no_tool_messages_passthrough() {
        let messages = vec![
            ChatCompletionMessage {
                role: "system".to_string(),
                content: Some(MessageContent::Text("You are helpful.".to_string())),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            },
            ChatCompletionMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello".to_string())),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            },
        ];
        let result = flatten_tool_messages(messages);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "system");
        assert_eq!(result[1].role, "user");
    }

    #[test]
    fn flatten_tool_call_and_result() {
        let messages = vec![
            ChatCompletionMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("test".to_string())),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            },
            ChatCompletionMessage {
                role: "assistant".to_string(),
                content: None,
                tool_call_id: None,
                name: None,
                tool_calls: Some(vec![tool_call("call_1", "echo", r#"{"message":"hi"}"#)]),
            },
            ChatCompletionMessage {
                role: "tool".to_string(),
                content: Some(MessageContent::Text("hi".to_string())),
                tool_call_id: Some("call_1".to_string()),
                name: Some("echo".to_string()),
                tool_calls: None,
            },
        ];

        let result = flatten_tool_messages(messages);
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].role, "user");
        assert!(result[1].tool_call_id.is_none());
        let tool_result_text = result[1]
            .content
            .as_ref()
            .and_then(|c| c.as_text())
            .unwrap();
        assert_eq!(tool_result_text, "Tool result from echo: hi");
    }

    #[test]
    fn flatten_applies_to_assistant_tool_calls_without_tool_result_messages() {
        let messages = vec![
            ChatCompletionMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("test".to_string())),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            },
            ChatCompletionMessage {
                role: "assistant".to_string(),
                content: None,
                tool_call_id: None,
                name: None,
                tool_calls: Some(vec![tool_call(
                    "call_1",
                    "demo__echo",
                    r#"{"message":"hi"}"#,
                )]),
            },
        ];

        let result = flatten_tool_messages(messages);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "user");
    }

    #[test]
    fn flatten_preserves_assistant_text_with_tool_calls() {
        let messages = vec![
            ChatCompletionMessage {
                role: "assistant".to_string(),
                content: Some(MessageContent::Text("Let me check that.".to_string())),
                tool_call_id: None,
                name: None,
                tool_calls: Some(vec![tool_call("call_1", "search", r#"{"q":"test"}"#)]),
            },
            ChatCompletionMessage {
                role: "tool".to_string(),
                content: Some(MessageContent::Text("found it".to_string())),
                tool_call_id: Some("call_1".to_string()),
                name: Some("search".to_string()),
                tool_calls: None,
            },
        ];

        let result = flatten_tool_messages(messages);
        assert_eq!(result.len(), 2);
        let text = result[0]
            .content
            .as_ref()
            .and_then(|c| c.as_text())
            .unwrap();
        assert_eq!(text, "Let me check that.");
        assert!(!text.contains("Called tool"));
        assert!(!text.contains("with arguments"));
        assert!(!text.contains(r#""q":"test""#));
        let result_text = result[1]
            .content
            .as_ref()
            .and_then(|c| c.as_text())
            .unwrap();
        assert_eq!(result_text, "Tool result from search: found it");
    }

    #[test]
    fn flatten_tool_result_missing_name_uses_unknown() {
        let messages = vec![ChatCompletionMessage {
            role: "tool".to_string(),
            content: Some(MessageContent::Text("result data".to_string())),
            tool_call_id: Some("call_1".to_string()),
            name: None,
            tool_calls: None,
        }];
        let result = flatten_tool_messages(messages);
        assert_eq!(result[0].role, "user");
        assert!(
            result[0]
                .content
                .as_ref()
                .unwrap()
                .as_text()
                .unwrap()
                .contains("Tool result from unknown:")
        );
    }

    #[test]
    fn flatten_tool_result_missing_content_uses_empty() {
        let messages = vec![ChatCompletionMessage {
            role: "tool".to_string(),
            content: None,
            tool_call_id: Some("call_1".to_string()),
            name: Some("my_tool".to_string()),
            tool_calls: None,
        }];
        let result = flatten_tool_messages(messages);
        assert_eq!(result[0].role, "user");
        assert!(
            result[0]
                .content
                .as_ref()
                .unwrap()
                .as_text()
                .unwrap()
                .contains("Tool result from my_tool:")
        );
    }

    #[test]
    fn flatten_multiple_tool_calls_in_single_assistant_message() {
        let messages = vec![
            ChatCompletionMessage {
                role: "assistant".to_string(),
                content: None,
                tool_call_id: None,
                name: None,
                tool_calls: Some(vec![
                    tool_call("call_1", "search", r#"{"q":"a"}"#),
                    tool_call("call_2", "fetch", r#"{"url":"http://x"}"#),
                ]),
            },
            ChatCompletionMessage {
                role: "tool".to_string(),
                content: Some(MessageContent::Text("found".to_string())),
                tool_call_id: Some("call_1".to_string()),
                name: Some("search".to_string()),
                tool_calls: None,
            },
            ChatCompletionMessage {
                role: "tool".to_string(),
                content: Some(MessageContent::Text("fetched".to_string())),
                tool_call_id: Some("call_2".to_string()),
                name: Some("fetch".to_string()),
                tool_calls: None,
            },
        ];
        let result = flatten_tool_messages(messages);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "user");
        assert_eq!(result[1].role, "user");
        let first_result = result[0].content.as_ref().unwrap().as_text().unwrap();
        assert_eq!(first_result, "Tool result from search: found");
        let second_result = result[1].content.as_ref().unwrap().as_text().unwrap();
        assert_eq!(second_result, "Tool result from fetch: fetched");
    }

    #[test]
    fn flatten_applied_on_text_only_path() {
        let messages = vec![
            ChatCompletionMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("run it".to_string())),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            },
            ChatCompletionMessage {
                role: "tool".to_string(),
                content: Some(MessageContent::Text("ok".to_string())),
                tool_call_id: Some("call_1".to_string()),
                name: Some("run_cmd".to_string()),
                tool_calls: None,
            },
        ];
        let flattened = flatten_tool_messages(messages);
        assert_eq!(flattened.len(), 2);
        assert_eq!(flattened[1].role, "user");
        let text = flattened[1]
            .content
            .as_ref()
            .and_then(|c| c.as_text())
            .unwrap();
        assert_eq!(text, "Tool result from run_cmd: ok");
    }

    #[test]
    fn no_flatten_when_no_tool_messages() {
        let messages = vec![
            ChatCompletionMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("hi".to_string())),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            },
            ChatCompletionMessage {
                role: "assistant".to_string(),
                content: Some(MessageContent::Text("hello".to_string())),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            },
        ];
        let result = flatten_tool_messages(messages);
        assert_eq!(result[0].role, "user");
        assert_eq!(result[1].role, "assistant");
    }

    fn tool_call(id: &str, name: &str, arguments: &str) -> ChatCompletionToolCall {
        ChatCompletionToolCall {
            id: id.to_string(),
            call_type: "function".to_string(),
            function: ChatCompletionToolCallFunction {
                name: name.to_string(),
                arguments: arguments.to_string(),
            },
        }
    }
}
