use crate::llm::{ChatMessage, FinishReason, Role, ToolCall, ToolDefinition};

#[cfg(test)]
use super::types::{
    OpenAiChatRequest, OpenAiChatResponse, OpenAiChoice, OpenAiContentPart, OpenAiFunction,
    OpenAiUsage,
};
use super::types::{
    OpenAiContent, OpenAiMessage, OpenAiTool, OpenAiToolCall, OpenAiToolCallFunction,
};

const MAX_MODEL_NAME_BYTES: usize = 256;

fn parse_role(s: &str) -> Result<Role, String> {
    match s {
        "system" => Ok(Role::System),
        // OpenAI newer models use "developer" role for high-priority instructions.
        // IronClaw maps it to our internal System role.
        "developer" => Ok(Role::System),
        "user" => Ok(Role::User),
        "assistant" => Ok(Role::Assistant),
        "tool" => Ok(Role::Tool),
        _ => Err(format!("Unknown role: '{}'", s)),
    }
}

fn content_to_text(
    content: Option<&OpenAiContent>,
    message_index: usize,
) -> Result<String, String> {
    match content {
        None => Ok(String::new()),
        Some(OpenAiContent::Text(s)) => Ok(s.clone()),
        Some(OpenAiContent::Parts(parts)) => {
            let mut out = String::new();
            for (part_index, part) in parts.iter().enumerate() {
                if let Some(text) = part.text.as_deref() {
                    out.push_str(text);
                    continue;
                }
                return Err(format!(
                    "messages[{}].content[{}]: unsupported content part type '{}'",
                    message_index, part_index, part.part_type
                ));
            }
            Ok(out)
        }
    }
}

pub(crate) fn convert_messages(messages: &[OpenAiMessage]) -> Result<Vec<ChatMessage>, String> {
    messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let role = parse_role(&m.role).map_err(|e| format!("messages[{}]: {}", i, e))?;
            let content = content_to_text(m.content.as_ref(), i)?;
            match role {
                Role::Tool => {
                    let tool_call_id = m.tool_call_id.as_deref().ok_or_else(|| {
                        format!("messages[{}]: tool message requires 'tool_call_id'", i)
                    })?;
                    let name = m
                        .name
                        .as_deref()
                        .ok_or_else(|| format!("messages[{}]: tool message requires 'name'", i))?;
                    Ok(ChatMessage::tool_result(tool_call_id, name, content))
                }
                Role::Assistant => {
                    if let Some(ref tcs) = m.tool_calls {
                        let calls: Vec<ToolCall> = tcs
                            .iter()
                            .enumerate()
                            .map(|(j, tc)| {
                                let arguments: serde_json::Value =
                                    serde_json::from_str(&tc.function.arguments).map_err(|e| {
                                        format!(
                                            "messages[{}].tool_calls[{}].function.arguments: invalid JSON: {}",
                                            i, j, e
                                        )
                                    })?;
                                Ok(ToolCall {
                                    id: tc.id.clone(),
                                    name: tc.function.name.clone(),
                                    arguments,
                                })
                            })
                            .collect::<Result<Vec<_>, String>>()?;
                        let assistant_content = if content.is_empty() {
                            None
                        } else {
                            Some(content)
                        };
                        Ok(ChatMessage::assistant_with_tool_calls(
                            assistant_content,
                            calls,
                        ))
                    } else {
                        Ok(ChatMessage::assistant(content))
                    }
                }
                _ => Ok(ChatMessage {
                    role,
                    content,
                    tool_call_id: None,
                    name: m.name.clone(),
                    tool_calls: None,
                }),
            }
        })
        .collect()
}

pub(crate) fn convert_tools(tools: &[OpenAiTool]) -> Vec<ToolDefinition> {
    tools
        .iter()
        .filter(|t| t.tool_type == "function")
        .map(|t| ToolDefinition {
            name: t.function.name.clone(),
            description: t.function.description.clone().unwrap_or_default(),
            parameters: t
                .function
                .parameters
                .clone()
                .unwrap_or(serde_json::json!({"type": "object", "properties": {}})),
        })
        .collect()
}

pub(crate) fn convert_tool_calls_to_openai(calls: &[ToolCall]) -> Vec<OpenAiToolCall> {
    calls
        .iter()
        .map(|tc| OpenAiToolCall {
            id: tc.id.clone(),
            call_type: "function".to_string(),
            function: OpenAiToolCallFunction {
                name: tc.name.clone(),
                arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
            },
        })
        .collect()
}

pub(crate) fn finish_reason_str(reason: FinishReason) -> String {
    match reason {
        FinishReason::Stop => "stop".to_string(),
        FinishReason::Length => "length".to_string(),
        FinishReason::ToolUse => "tool_calls".to_string(),
        FinishReason::ContentFilter => "content_filter".to_string(),
        FinishReason::Unknown => "stop".to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolChoice {
    Mode(String),
    NamedFunction(String),
}

pub(crate) fn normalize_tool_choice(val: &serde_json::Value) -> Result<Option<ToolChoice>, String> {
    match val {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(s) => match s.as_str() {
            "auto" | "required" | "none" => Ok(Some(ToolChoice::Mode(s.clone()))),
            other => Err(format!(
                "tool_choice must be one of 'auto', 'required', or 'none'; got '{}'",
                other
            )),
        },
        serde_json::Value::Object(obj) => match obj.get("type").and_then(|v| v.as_str()) {
            Some("auto" | "required" | "none") => Ok(Some(ToolChoice::Mode(
                obj.get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            ))),
            Some("function") => {
                let name = obj
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| {
                        "tool_choice.function.name must be a non-empty string".to_string()
                    })?;
                Ok(Some(ToolChoice::NamedFunction(name.to_string())))
            }
            Some(other) => Err(format!(
                "tool_choice.type must be 'auto', 'required', 'none', or 'function'; got '{}'",
                other
            )),
            None => Err("tool_choice object must include a 'type' field".to_string()),
        },
        _ => Err("tool_choice must be a string or object".to_string()),
    }
}

pub(crate) fn apply_named_tool_choice(
    tools: Vec<ToolDefinition>,
    tool_choice: Option<ToolChoice>,
) -> Result<(Vec<ToolDefinition>, Option<String>), String> {
    match tool_choice {
        None => Ok((tools, None)),
        Some(ToolChoice::Mode(mode)) => Ok((tools, Some(mode))),
        Some(ToolChoice::NamedFunction(name)) => {
            let filtered: Vec<ToolDefinition> =
                tools.into_iter().filter(|t| t.name == name).collect();
            if filtered.is_empty() {
                return Err(format!(
                    "tool_choice.function.name '{}' does not match any provided tool",
                    name
                ));
            }
            // Preserve named function semantics by narrowing tools to the named
            // function and requiring a tool call.
            Ok((filtered, Some("required".to_string())))
        }
    }
}

pub(crate) fn validate_model_name(model: &str) -> Result<(), String> {
    let trimmed = model.trim();

    if trimmed.is_empty() {
        return Err("model must not be empty".to_string());
    }
    if trimmed != model {
        return Err("model must not have leading or trailing whitespace".to_string());
    }
    if model.len() > MAX_MODEL_NAME_BYTES {
        return Err(format!(
            "model must be at most {} bytes",
            MAX_MODEL_NAME_BYTES
        ));
    }
    if model.chars().any(char::is_control) {
        return Err("model contains control characters".to_string());
    }
    Ok(())
}

/// Extract stop sequences from the flexible `stop` field.
pub(crate) fn parse_stop(val: &serde_json::Value) -> Option<Vec<String>> {
    match val {
        serde_json::Value::String(s) => Some(vec![s.clone()]),
        serde_json::Value::Array(arr) => {
            let strs: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if strs.is_empty() { None } else { Some(strs) }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_content(text: &str) -> Option<OpenAiContent> {
        Some(OpenAiContent::Text(text.to_string()))
    }

    #[test]
    fn test_parse_role() {
        assert_eq!(parse_role("system").unwrap(), Role::System);
        assert_eq!(parse_role("developer").unwrap(), Role::System);
        assert_eq!(parse_role("user").unwrap(), Role::User);
        assert_eq!(parse_role("assistant").unwrap(), Role::Assistant);
        assert_eq!(parse_role("tool").unwrap(), Role::Tool);
    }

    #[test]
    fn test_parse_role_unknown_rejected() {
        let err = parse_role("unknown").unwrap_err();
        assert!(err.contains("Unknown role"));
        assert!(err.contains("unknown"));
    }

    #[test]
    fn test_finish_reason_str() {
        assert_eq!(finish_reason_str(FinishReason::Stop), "stop");
        assert_eq!(finish_reason_str(FinishReason::Length), "length");
        assert_eq!(finish_reason_str(FinishReason::ToolUse), "tool_calls");
        assert_eq!(
            finish_reason_str(FinishReason::ContentFilter),
            "content_filter"
        );
        assert_eq!(finish_reason_str(FinishReason::Unknown), "stop");
    }

    #[test]
    fn test_convert_messages_basic() {
        let msgs = vec![
            OpenAiMessage {
                role: "system".to_string(),
                content: text_content("You are helpful."),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
            OpenAiMessage {
                role: "user".to_string(),
                content: text_content("Hello"),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
        ];

        let converted = convert_messages(&msgs).unwrap();
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0].role, Role::System);
        assert_eq!(converted[0].content, "You are helpful.");
        assert_eq!(converted[1].role, Role::User);
        assert_eq!(converted[1].content, "Hello");
    }

    #[test]
    fn test_convert_messages_with_tool_results() {
        let msgs = vec![OpenAiMessage {
            role: "tool".to_string(),
            content: text_content("42"),
            name: Some("calculator".to_string()),
            tool_call_id: Some("call_123".to_string()),
            tool_calls: None,
        }];

        let converted = convert_messages(&msgs).unwrap();
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, Role::Tool);
        assert_eq!(converted[0].content, "42");
        assert_eq!(converted[0].tool_call_id.as_deref(), Some("call_123"));
        assert_eq!(converted[0].name.as_deref(), Some("calculator"));
    }

    #[test]
    fn test_convert_tools() {
        let tools = vec![OpenAiTool {
            tool_type: "function".to_string(),
            function: OpenAiFunction {
                name: "get_weather".to_string(),
                description: Some("Get weather for a location".to_string()),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "location": { "type": "string" }
                    },
                    "required": ["location"]
                })),
            },
        }];

        let converted = convert_tools(&tools);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].name, "get_weather");
        assert_eq!(converted[0].description, "Get weather for a location");
    }

    #[test]
    fn test_convert_tool_calls_to_openai() {
        let calls = vec![ToolCall {
            id: "call_abc".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "rust"}),
        }];

        let converted = convert_tool_calls_to_openai(&calls);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].id, "call_abc");
        assert_eq!(converted[0].call_type, "function");
        assert_eq!(converted[0].function.name, "search");
        assert!(converted[0].function.arguments.contains("rust"));
    }

    #[test]
    fn test_normalize_tool_choice() {
        // String variant
        let v = serde_json::json!("auto");
        assert_eq!(
            normalize_tool_choice(&v).unwrap(),
            Some(ToolChoice::Mode("auto".to_string()))
        );

        // Named function object
        let v = serde_json::json!({"type": "function", "function": {"name": "foo"}});
        assert_eq!(
            normalize_tool_choice(&v).unwrap(),
            Some(ToolChoice::NamedFunction("foo".to_string()))
        );

        // Object with type only
        let v = serde_json::json!({"type": "none"});
        assert_eq!(
            normalize_tool_choice(&v).unwrap(),
            Some(ToolChoice::Mode("none".to_string()))
        );

        // Null
        let v = serde_json::Value::Null;
        assert_eq!(normalize_tool_choice(&v).unwrap(), None);
    }

    #[test]
    fn test_apply_named_tool_choice_filters_to_named_tool() {
        let tools = vec![
            ToolDefinition {
                name: "get_weather".to_string(),
                description: "Get weather".to_string(),
                parameters: serde_json::json!({"type":"object","properties":{}}),
            },
            ToolDefinition {
                name: "search_web".to_string(),
                description: "Search web".to_string(),
                parameters: serde_json::json!({"type":"object","properties":{}}),
            },
        ];

        let (filtered, choice) = apply_named_tool_choice(
            tools,
            Some(ToolChoice::NamedFunction("search_web".to_string())),
        )
        .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "search_web");
        assert_eq!(choice.as_deref(), Some("required"));
    }

    #[test]
    fn test_convert_messages_developer_role_maps_to_system() {
        let msgs = vec![OpenAiMessage {
            role: "developer".to_string(),
            content: text_content("Follow these rules"),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }];
        let converted = convert_messages(&msgs).unwrap();
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, Role::System);
        assert_eq!(converted[0].content, "Follow these rules");
    }

    #[test]
    fn test_convert_messages_array_content_parts() {
        let msgs = vec![OpenAiMessage {
            role: "user".to_string(),
            content: Some(OpenAiContent::Parts(vec![
                OpenAiContentPart {
                    part_type: "text".to_string(),
                    text: Some("Hello ".to_string()),
                },
                OpenAiContentPart {
                    part_type: "text".to_string(),
                    text: Some("world".to_string()),
                },
            ])),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }];
        let converted = convert_messages(&msgs).unwrap();
        assert_eq!(converted[0].content, "Hello world");
    }

    #[test]
    fn test_convert_messages_invalid_tool_arguments_error() {
        let msgs = vec![OpenAiMessage {
            role: "assistant".to_string(),
            content: None,
            name: None,
            tool_call_id: None,
            tool_calls: Some(vec![OpenAiToolCall {
                id: "call_bad".to_string(),
                call_type: "function".to_string(),
                function: OpenAiToolCallFunction {
                    name: "bad_tool".to_string(),
                    arguments: "{not-json".to_string(),
                },
            }]),
        }];
        let err = convert_messages(&msgs).unwrap_err();
        assert!(err.contains("function.arguments"));
        assert!(err.contains("invalid JSON"));
    }

    #[test]
    fn test_openai_request_deserialize_minimal() {
        let json = r#"{"model":"gpt-4","messages":[{"role":"user","content":"Hi"}]}"#;
        let req: OpenAiChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.model, "gpt-4");
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.stream, None);
        assert_eq!(req.temperature, None);
    }

    #[test]
    fn test_openai_request_deserialize_streaming() {
        let json = r#"{"model":"gpt-4","messages":[{"role":"user","content":"Hi"}],"stream":true,"temperature":0.7}"#;
        let req: OpenAiChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.stream, Some(true));
        assert_eq!(req.temperature, Some(0.7));
    }

    #[test]
    fn test_openai_response_serialize() {
        let resp = OpenAiChatResponse {
            id: "chatcmpl-test".to_string(),
            object: "chat.completion",
            created: 1234567890,
            model: "test-model".to_string(),
            choices: vec![OpenAiChoice {
                index: 0,
                message: OpenAiMessage {
                    role: "assistant".to_string(),
                    content: text_content("Hello!"),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                finish_reason: "stop".to_string(),
            }],
            usage: OpenAiUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
        };

        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["object"], "chat.completion");
        assert_eq!(json["choices"][0]["finish_reason"], "stop");
        assert_eq!(json["choices"][0]["message"]["content"], "Hello!");
        assert_eq!(json["usage"]["total_tokens"], 15);
    }

    #[test]
    fn test_openai_message_with_null_content() {
        let json = r#"{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"search","arguments":"{\"q\":\"test\"}"}}]}"#;
        let msg: OpenAiMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.role, "assistant");
        assert!(msg.content.is_none());
        assert!(msg.tool_calls.is_some());
        assert_eq!(msg.tool_calls.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_convert_messages_unknown_role_rejected() {
        let msgs = vec![OpenAiMessage {
            role: "moderator".to_string(),
            content: text_content("Hi"),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }];
        let err = convert_messages(&msgs).unwrap_err();
        assert!(err.contains("messages[0]"));
        assert!(err.contains("Unknown role"));
    }

    #[test]
    fn test_convert_messages_tool_missing_fields() {
        // Missing tool_call_id
        let msgs = vec![OpenAiMessage {
            role: "tool".to_string(),
            content: text_content("result"),
            name: Some("calc".to_string()),
            tool_call_id: None,
            tool_calls: None,
        }];
        let err = convert_messages(&msgs).unwrap_err();
        assert!(err.contains("tool_call_id"));

        // Missing name
        let msgs = vec![OpenAiMessage {
            role: "tool".to_string(),
            content: text_content("result"),
            name: None,
            tool_call_id: Some("call_1".to_string()),
            tool_calls: None,
        }];
        let err = convert_messages(&msgs).unwrap_err();
        assert!(err.contains("'name'"));
    }

    #[test]
    fn test_parse_stop_string() {
        let v = serde_json::json!("STOP");
        assert_eq!(parse_stop(&v), Some(vec!["STOP".to_string()]));
    }

    #[test]
    fn test_parse_stop_array() {
        let v = serde_json::json!(["STOP", "END"]);
        assert_eq!(
            parse_stop(&v),
            Some(vec!["STOP".to_string(), "END".to_string()])
        );
    }

    #[test]
    fn test_parse_stop_null() {
        let v = serde_json::Value::Null;
        assert_eq!(parse_stop(&v), None);
    }

    #[test]
    fn test_validate_model_name_rejects_leading_or_trailing_whitespace() {
        let err = validate_model_name(" gpt-4").unwrap_err();
        assert!(err.contains("leading or trailing whitespace"));

        let err = validate_model_name("gpt-4 ").unwrap_err();
        assert!(err.contains("leading or trailing whitespace"));
    }

    #[test]
    fn test_validate_model_name_accepts_normal_name() {
        assert!(validate_model_name("gpt-4").is_ok());
    }
}
