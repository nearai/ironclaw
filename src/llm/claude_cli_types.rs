//! Shared NDJSON types for Claude CLI streaming output.
//!
//! These types are shared between:
//! - `src/worker/claude_bridge.rs` (sandbox container bridge)
//! - `src/llm/claude_cli.rs` (host-side LLM provider)
//!
//! Claude Code emits one JSON object per line with `--output-format stream-json`:
//!
//!   system    -> session init (session_id, tools, model)
//!   assistant -> LLM response, nested under message.content[] as text/tool_use blocks
//!   user      -> tool results, nested under message.content[] as tool_result blocks
//!   result    -> final summary (is_error, duration_ms, num_turns, result text)

use serde::{Deserialize, Serialize};

/// A Claude Code streaming event (NDJSON line from `--output-format stream-json`).
///
/// Content blocks live under `message.content`, NOT at the top level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeStreamEvent {
    #[serde(rename = "type")]
    pub event_type: String,

    #[serde(default)]
    pub session_id: Option<String>,

    #[serde(default)]
    pub subtype: Option<String>,

    /// For `assistant` and `user` events: the message wrapper containing content blocks.
    #[serde(default)]
    pub message: Option<MessageWrapper>,

    /// For `result` events: the final text output.
    #[serde(default)]
    pub result: Option<serde_json::Value>,

    /// For `result` events: whether the session ended in error.
    #[serde(default)]
    pub is_error: Option<bool>,

    /// For `result` events: total wall-clock duration.
    #[serde(default)]
    pub duration_ms: Option<u64>,

    /// For `result` events: number of agentic turns used.
    #[serde(default)]
    pub num_turns: Option<u32>,

    /// Usage information (tokens consumed). Present on assistant events.
    #[serde(default)]
    pub usage: Option<UsageInfo>,
}

/// Wrapper around the `message` field in assistant/user events.
///
/// ```text
/// { "type": "assistant", "message": { "content": [ { "type": "text", ... } ] } }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageWrapper {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<Vec<ContentBlock>>,
    /// Usage info sometimes appears at the message level.
    #[serde(default)]
    pub usage: Option<UsageInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    /// Text block content.
    #[serde(default)]
    pub text: Option<String>,
    /// Tool name (for tool_use blocks).
    #[serde(default)]
    pub name: Option<String>,
    /// Tool use ID (for tool_use and tool_result blocks).
    #[serde(default)]
    pub id: Option<String>,
    /// Tool input params (for tool_use blocks).
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    /// Tool result content (for tool_result blocks), or general content.
    #[serde(default)]
    pub content: Option<serde_json::Value>,
    /// Tool use ID reference (for tool_result blocks).
    #[serde(default)]
    pub tool_use_id: Option<String>,
}

/// Token usage information from Claude CLI output.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageInfo {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u32>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_system_event() {
        let json = r#"{"type":"system","session_id":"abc-123","subtype":"init"}"#;
        let event: ClaudeStreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_type, "system");
        assert_eq!(event.session_id.as_deref(), Some("abc-123"));
        assert!(event.message.is_none());
        assert!(event.usage.is_none());
    }

    #[test]
    fn test_parse_assistant_text_event() {
        let json = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hello world"}]}}"#;
        let event: ClaudeStreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_type, "assistant");
        let blocks = event.message.unwrap().content.unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].block_type, "text");
        assert_eq!(blocks[0].text.as_deref(), Some("Hello world"));
    }

    #[test]
    fn test_parse_assistant_tool_use_event() {
        let json = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_01abc","name":"Bash","input":{"command":"ls"}}]}}"#;
        let event: ClaudeStreamEvent = serde_json::from_str(json).unwrap();
        let blocks = event.message.unwrap().content.unwrap();
        assert_eq!(blocks[0].block_type, "tool_use");
        assert_eq!(blocks[0].name.as_deref(), Some("Bash"));
        assert_eq!(blocks[0].id.as_deref(), Some("toolu_01abc"));
        assert!(blocks[0].input.is_some());
    }

    #[test]
    fn test_parse_user_tool_result_event() {
        let json = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01abc","content":"/workspace"}]}}"#;
        let event: ClaudeStreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_type, "user");
        let blocks = event.message.unwrap().content.unwrap();
        assert_eq!(blocks[0].block_type, "tool_result");
        assert_eq!(blocks[0].tool_use_id.as_deref(), Some("toolu_01abc"));
    }

    #[test]
    fn test_parse_result_event() {
        let json = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":5000,"num_turns":3,"result":"Done.","session_id":"sid-1"}"#;
        let event: ClaudeStreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_type, "result");
        assert_eq!(event.is_error, Some(false));
        assert_eq!(event.duration_ms, Some(5000));
        assert_eq!(event.num_turns, Some(3));
        assert_eq!(event.result.unwrap().as_str().unwrap(), "Done.");
    }

    #[test]
    fn test_parse_result_error_event() {
        let json = r#"{"type":"result","subtype":"error_max_turns","is_error":true,"duration_ms":60000,"num_turns":50}"#;
        let event: ClaudeStreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.is_error, Some(true));
        assert_eq!(event.subtype.as_deref(), Some("error_max_turns"));
    }

    #[test]
    fn test_parse_usage_info() {
        let json = r#"{"input_tokens":100,"output_tokens":50,"cache_creation_input_tokens":10,"cache_read_input_tokens":5}"#;
        let usage: UsageInfo = serde_json::from_str(json).unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.cache_creation_input_tokens, Some(10));
        assert_eq!(usage.cache_read_input_tokens, Some(5));
    }

    #[test]
    fn test_parse_usage_info_defaults() {
        let json = r#"{}"#;
        let usage: UsageInfo = serde_json::from_str(json).unwrap();
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
        assert!(usage.cache_creation_input_tokens.is_none());
    }

    #[test]
    fn test_parse_assistant_event_with_usage() {
        let json = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hi"}],"usage":{"input_tokens":50,"output_tokens":25}}}"#;
        let event: ClaudeStreamEvent = serde_json::from_str(json).unwrap();
        let msg = event.message.unwrap();
        let usage = msg.usage.unwrap();
        assert_eq!(usage.input_tokens, 50);
        assert_eq!(usage.output_tokens, 25);
    }

    #[test]
    fn test_parse_event_with_top_level_usage() {
        let json = r#"{"type":"assistant","usage":{"input_tokens":100,"output_tokens":200},"message":{"content":[{"type":"text","text":"test"}]}}"#;
        let event: ClaudeStreamEvent = serde_json::from_str(json).unwrap();
        let usage = event.usage.unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 200);
    }

    #[test]
    fn test_unknown_fields_ignored() {
        let json = r#"{"type":"system","session_id":"s1","unknown_field":"value","another":42}"#;
        let event: ClaudeStreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_type, "system");
        assert_eq!(event.session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn test_missing_optional_fields_default_to_none() {
        let json = r#"{"type":"assistant"}"#;
        let event: ClaudeStreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_type, "assistant");
        assert!(event.session_id.is_none());
        assert!(event.subtype.is_none());
        assert!(event.message.is_none());
        assert!(event.result.is_none());
        assert!(event.is_error.is_none());
        assert!(event.duration_ms.is_none());
        assert!(event.num_turns.is_none());
        assert!(event.usage.is_none());
    }

    #[test]
    fn test_content_block_serde_roundtrip() {
        let block = ContentBlock {
            block_type: "tool_use".to_string(),
            text: None,
            name: Some("Bash".to_string()),
            id: Some("toolu_123".to_string()),
            input: Some(serde_json::json!({"command": "ls -la"})),
            content: None,
            tool_use_id: None,
        };
        let json = serde_json::to_string(&block).unwrap();
        let parsed: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.block_type, "tool_use");
        assert_eq!(parsed.name.as_deref(), Some("Bash"));
        assert_eq!(parsed.id.as_deref(), Some("toolu_123"));
    }

    #[test]
    fn test_full_event_serde_roundtrip() {
        let event = ClaudeStreamEvent {
            event_type: "result".to_string(),
            session_id: Some("sess-abc".to_string()),
            subtype: Some("success".to_string()),
            message: None,
            result: Some(serde_json::json!("All done.")),
            is_error: Some(false),
            duration_ms: Some(3000),
            num_turns: Some(2),
            usage: Some(UsageInfo {
                input_tokens: 500,
                output_tokens: 200,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: Some(100),
            }),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: ClaudeStreamEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event_type, "result");
        assert_eq!(parsed.session_id.as_deref(), Some("sess-abc"));
        assert_eq!(parsed.is_error, Some(false));
        let usage = parsed.usage.unwrap();
        assert_eq!(usage.input_tokens, 500);
        assert_eq!(usage.output_tokens, 200);
    }

    #[test]
    fn test_mixed_content_blocks() {
        let json = r#"{"type":"assistant","message":{"content":[
            {"type":"text","text":"Let me check that."},
            {"type":"tool_use","id":"toolu_01","name":"Bash","input":{"command":"ls"}},
            {"type":"text","text":"Here are the results."}
        ]}}"#;
        let event: ClaudeStreamEvent = serde_json::from_str(json).unwrap();
        let blocks = event.message.unwrap().content.unwrap();
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].block_type, "text");
        assert_eq!(blocks[1].block_type, "tool_use");
        assert_eq!(blocks[2].block_type, "text");
    }
}
