//! Strongly typed NDJSON events from Claude CLI.
//!
//! Claude Code emits one JSON object per line when run with `--output-format stream-json`.
//! This module provides a typed `CliEvent` enum that replaces the flat `ClaudeStreamEvent`
//! struct with proper variants for each event type.

use serde::{Deserialize, Serialize};

use crate::claude_cli::error::ClaudeCliError;

/// A parsed event from Claude CLI NDJSON output.
#[derive(Debug, Clone)]
pub enum CliEvent {
    /// System init event with session info and available tools.
    System(SystemEvent),

    /// Assistant message with content blocks (text, tool_use).
    Assistant(AssistantEvent),

    /// User/tool result message.
    User(UserEvent),

    /// Final result event with cost and duration info.
    Result(ResultEvent),

    /// Unknown event type (forward compatibility).
    Unknown {
        event_type: String,
        raw: serde_json::Value,
    },
}

/// System init event emitted at the start of a Claude CLI session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemEvent {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub subtype: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tools: Option<Vec<ToolInfo>>,
}

/// Tool info from the system init event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// Assistant message event containing content blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantEvent {
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub session_id: Option<String>,
}

/// User or tool result message event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserEvent {
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub session_id: Option<String>,
}

/// Typed content block from Claude CLI output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },

    #[serde(rename = "tool_use")]
    ToolUse {
        #[serde(default)]
        id: Option<String>,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },

    #[serde(rename = "tool_result")]
    ToolResult {
        #[serde(default)]
        tool_use_id: Option<String>,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        is_error: Option<bool>,
    },
}

/// Final result event with session outcome and cost info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultEvent {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub subtype: Option<String>,
    #[serde(default)]
    pub result: Option<ResultInfo>,
    #[serde(default)]
    pub is_error: Option<bool>,
}

/// Detailed result info nested within a result event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultInfo {
    #[serde(default)]
    pub is_error: Option<bool>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub duration_api_ms: Option<u64>,
    #[serde(default)]
    pub num_turns: Option<u32>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
}

impl CliEvent {
    /// Parse a single NDJSON line into a `CliEvent`.
    ///
    /// Handles unknown event types gracefully by wrapping them in `Unknown`.
    /// Unknown content block types within assistant events are silently skipped.
    pub fn parse(line: &str) -> Result<Self, ClaudeCliError> {
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|e| ClaudeCliError::ParseError {
                reason: e.to_string(),
                raw: line.to_string(),
            })?;

        let event_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        match event_type.as_str() {
            "system" => {
                let evt: SystemEvent =
                    serde_json::from_value(value).map_err(|e| ClaudeCliError::ParseError {
                        reason: e.to_string(),
                        raw: line.to_string(),
                    })?;
                Ok(CliEvent::System(evt))
            }
            "assistant" => {
                let evt = deserialize_message_event(&value);
                Ok(CliEvent::Assistant(AssistantEvent {
                    content: evt.0,
                    session_id: evt.1,
                }))
            }
            "user" => {
                let evt = deserialize_message_event(&value);
                Ok(CliEvent::User(UserEvent {
                    content: evt.0,
                    session_id: evt.1,
                }))
            }
            "result" => {
                let evt: ResultEvent =
                    serde_json::from_value(value).map_err(|e| ClaudeCliError::ParseError {
                        reason: e.to_string(),
                        raw: line.to_string(),
                    })?;
                Ok(CliEvent::Result(evt))
            }
            other => Ok(CliEvent::Unknown {
                event_type: other.to_string(),
                raw: value,
            }),
        }
    }

    /// Returns the session_id if present on this event.
    pub fn session_id(&self) -> Option<&str> {
        match self {
            CliEvent::System(e) => e.session_id.as_deref(),
            CliEvent::Assistant(e) => e.session_id.as_deref(),
            CliEvent::User(e) => e.session_id.as_deref(),
            CliEvent::Result(e) => e.session_id.as_deref(),
            CliEvent::Unknown { .. } => None,
        }
    }

    /// Returns true if this is a result event.
    pub fn is_result(&self) -> bool {
        matches!(self, CliEvent::Result(_))
    }

    /// Returns true if this is a result event indicating an error.
    pub fn is_error(&self) -> bool {
        match self {
            CliEvent::Result(r) => {
                r.is_error.unwrap_or(false)
                    || r.result
                        .as_ref()
                        .and_then(|ri| ri.is_error)
                        .unwrap_or(false)
            }
            _ => false,
        }
    }
}

/// Deserialize content blocks from an assistant or user event, skipping unknown block types.
fn deserialize_message_event(value: &serde_json::Value) -> (Vec<ContentBlock>, Option<String>) {
    let session_id = value
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(String::from);

    let content_arr = value
        .get("content")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut blocks = Vec::new();
    for block_val in content_arr {
        match serde_json::from_value::<ContentBlock>(block_val) {
            Ok(block) => blocks.push(block),
            Err(e) => {
                tracing::debug!("Skipping unknown content block type: {}", e);
            }
        }
    }

    (blocks, session_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_system_event() {
        let json = r#"{"type":"system","session_id":"abc-123","subtype":"init"}"#;
        let event = CliEvent::parse(json).unwrap();
        match event {
            CliEvent::System(sys) => {
                assert_eq!(sys.session_id.as_deref(), Some("abc-123"));
                assert_eq!(sys.subtype.as_deref(), Some("init"));
            }
            other => panic!("expected System, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_assistant_text_event() {
        let json = r#"{"type":"assistant","content":[{"type":"text","text":"Hello world"}]}"#;
        let event = CliEvent::parse(json).unwrap();
        match event {
            CliEvent::Assistant(asst) => {
                assert_eq!(asst.content.len(), 1);
                match &asst.content[0] {
                    ContentBlock::Text { text } => assert_eq!(text, "Hello world"),
                    other => panic!("expected Text, got {:?}", other),
                }
            }
            other => panic!("expected Assistant, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_assistant_tool_use_event() {
        let json = r#"{"type":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}"#;
        let event = CliEvent::parse(json).unwrap();
        match event {
            CliEvent::Assistant(asst) => {
                assert_eq!(asst.content.len(), 1);
                match &asst.content[0] {
                    ContentBlock::ToolUse { name, input, .. } => {
                        assert_eq!(name, "Bash");
                        assert_eq!(input["command"], "ls");
                    }
                    other => panic!("expected ToolUse, got {:?}", other),
                }
            }
            other => panic!("expected Assistant, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_result_event() {
        let json =
            r#"{"type":"result","result":{"is_error":false,"duration_ms":5000,"num_turns":3}}"#;
        let event = CliEvent::parse(json).unwrap();
        match event {
            CliEvent::Result(res) => {
                let info = res.result.unwrap();
                assert_eq!(info.is_error, Some(false));
                assert_eq!(info.duration_ms, Some(5000));
                assert_eq!(info.num_turns, Some(3));
            }
            other => panic!("expected Result, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_result_error_event() {
        let json = r#"{"type":"result","result":{"is_error":true}}"#;
        let event = CliEvent::parse(json).unwrap();
        assert!(event.is_result());
        assert!(event.is_error());
    }

    #[test]
    fn test_parse_unknown_event_type() {
        let json = r#"{"type":"fancy_new_thing","data":42}"#;
        let event = CliEvent::parse(json).unwrap();
        match event {
            CliEvent::Unknown { event_type, raw } => {
                assert_eq!(event_type, "fancy_new_thing");
                assert_eq!(raw["data"], 42);
            }
            other => panic!("expected Unknown, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_assistant_with_unknown_content_block() {
        let json = r#"{"type":"assistant","content":[{"type":"text","text":"hi"},{"type":"image","url":"http://example.com/img.png"}]}"#;
        let event = CliEvent::parse(json).unwrap();
        match event {
            CliEvent::Assistant(asst) => {
                // Unknown "image" block should be skipped
                assert_eq!(asst.content.len(), 1);
                match &asst.content[0] {
                    ContentBlock::Text { text } => assert_eq!(text, "hi"),
                    other => panic!("expected Text, got {:?}", other),
                }
            }
            other => panic!("expected Assistant, got {:?}", other),
        }
    }

    #[test]
    fn test_session_id_accessor() {
        let json = r#"{"type":"system","session_id":"sid-1"}"#;
        let event = CliEvent::parse(json).unwrap();
        assert_eq!(event.session_id(), Some("sid-1"));

        let json = r#"{"type":"assistant","content":[]}"#;
        let event = CliEvent::parse(json).unwrap();
        assert_eq!(event.session_id(), None);
    }

    #[test]
    fn test_is_result() {
        let json = r#"{"type":"result","result":{}}"#;
        let event = CliEvent::parse(json).unwrap();
        assert!(event.is_result());

        let json = r#"{"type":"assistant","content":[]}"#;
        let event = CliEvent::parse(json).unwrap();
        assert!(!event.is_result());
    }

    #[test]
    fn test_content_block_serde_roundtrip() {
        let block = ContentBlock::Text {
            text: "hello".to_string(),
        };
        let json = serde_json::to_string(&block).unwrap();
        let parsed: ContentBlock = serde_json::from_str(&json).unwrap();
        match parsed {
            ContentBlock::Text { text } => assert_eq!(text, "hello"),
            other => panic!("expected Text, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_invalid_json() {
        let result = CliEvent::parse("not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_tool_result_block() {
        let json = r#"{"type":"assistant","content":[{"type":"tool_result","tool_use_id":"tu_1","content":"output text","is_error":false}]}"#;
        let event = CliEvent::parse(json).unwrap();
        match event {
            CliEvent::Assistant(asst) => {
                assert_eq!(asst.content.len(), 1);
                match &asst.content[0] {
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => {
                        assert_eq!(tool_use_id.as_deref(), Some("tu_1"));
                        assert_eq!(content.as_deref(), Some("output text"));
                        assert_eq!(*is_error, Some(false));
                    }
                    other => panic!("expected ToolResult, got {:?}", other),
                }
            }
            other => panic!("expected Assistant, got {:?}", other),
        }
    }

    #[test]
    fn test_result_event_top_level_is_error() {
        let json = r#"{"type":"result","is_error":true}"#;
        let event = CliEvent::parse(json).unwrap();
        assert!(event.is_error());
    }
}
