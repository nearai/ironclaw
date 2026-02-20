//! Claude CLI LLM provider.
//!
//! Wraps the official `claude` CLI binary as an LLM backend for users with
//! Claude Max/Pro subscriptions. This avoids per-token API costs by using
//! the subscription's included usage.
//!
//! The CLI is invoked with `--output-format stream-json` and its NDJSON output
//! is parsed to extract text responses, tool calls, and usage information.
//!
//! Multi-turn conversations use `--resume <session_id>` to maintain context
//! without resending the full conversation history.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::config::ClaudeCliConfig;
use crate::error::LlmError;
use crate::llm::claude_cli_types::{ClaudeStreamEvent, UsageInfo};
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse, ToolDefinition,
};

/// Parsed output from a Claude CLI invocation.
#[derive(Debug, Default)]
struct CliOutput {
    /// Captured session ID from the `system` event.
    session_id: Option<String>,
    /// Aggregated text content from assistant events.
    text_content: String,
    /// Tool calls extracted from assistant events.
    tool_calls: Vec<ToolCall>,
    /// Whether the result event indicated an error.
    is_error: bool,
    /// Aggregated token usage across all events.
    input_tokens: u32,
    output_tokens: u32,
    /// Result text from the `result` event (fallback content).
    result_text: Option<String>,
}

/// Per-thread session tracking state.
struct CliSessionState {
    session_id: String,
    /// Number of messages that were included in the request that produced this session.
    input_count: usize,
}

/// Claude CLI LLM provider.
///
/// Spawns the `claude` binary as a subprocess for each request and parses
/// its NDJSON streaming output. Supports multi-turn conversations via
/// `--resume <session_id>`.
pub struct ClaudeCliProvider {
    config: ClaudeCliConfig,
    active_model: RwLock<String>,
    /// Per-thread session state for `--resume` support.
    sessions: RwLock<HashMap<String, CliSessionState>>,
}

impl ClaudeCliProvider {
    /// Create a new Claude CLI provider.
    pub fn new(config: ClaudeCliConfig) -> Self {
        let active_model = RwLock::new(config.model.clone());
        Self {
            config,
            active_model,
            sessions: RwLock::new(HashMap::new()),
        }
    }

    /// Store a session ID for a thread.
    fn store_session(&self, thread_id: &str, session_id: String, input_count: usize) {
        let mut sessions = match self.sessions.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::warn!("sessions lock poisoned in store; recovering");
                poisoned.into_inner()
            }
        };
        sessions.insert(
            thread_id.to_string(),
            CliSessionState {
                session_id,
                input_count,
            },
        );
    }

    /// Get the stored session ID for a thread.
    fn get_session(&self, thread_id: &str) -> Option<(String, usize)> {
        let sessions = match self.sessions.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::warn!("sessions lock poisoned in get; recovering");
                poisoned.into_inner()
            }
        };
        sessions
            .get(thread_id)
            .map(|s| (s.session_id.clone(), s.input_count))
    }

    /// Clear the session for a thread (used on resume failure).
    fn clear_session(&self, thread_id: &str) {
        let mut sessions = match self.sessions.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::warn!("sessions lock poisoned in clear; recovering");
                poisoned.into_inner()
            }
        };
        sessions.remove(thread_id);
    }

    /// Get the currently active model name.
    fn current_model(&self) -> String {
        self.active_model
            .read()
            .map(|m| m.clone())
            .unwrap_or_else(|p| p.into_inner().clone())
    }

    /// Spawn the `claude` CLI and parse its output.
    async fn spawn_claude(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        tools: &[ToolDefinition],
        resume_session_id: Option<&str>,
    ) -> Result<CliOutput, LlmError> {
        let model = self.current_model();

        let mut cmd = Command::new(&self.config.binary_path);
        cmd.arg("-p")
            .arg(prompt)
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--max-turns")
            .arg(self.config.max_turns.to_string())
            .arg("--model")
            .arg(&model);

        if let Some(system) = system_prompt {
            cmd.arg("--system-prompt").arg(system);
        }

        // Pass tool definitions via --allowedTools so the CLI knows which
        // tools IronClaw will handle.
        if !tools.is_empty() {
            let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
            tracing::debug!(tools = ?tool_names, "Passing tool definitions to CLI");
        }

        if let Some(sid) = resume_session_id {
            cmd.arg("--resume").arg(sid);
        }

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                LlmError::RequestFailed {
                    provider: "claude_cli".to_string(),
                    reason: format!(
                        "Claude CLI binary '{}' not found. Install it from https://docs.anthropic.com/en/docs/claude-code \
                         and authenticate with `claude auth login`.",
                        self.config.binary_path
                    ),
                }
            } else {
                LlmError::RequestFailed {
                    provider: "claude_cli".to_string(),
                    reason: format!("Failed to spawn claude CLI: {}", e),
                }
            }
        })?;

        let stdout = child.stdout.take().ok_or_else(|| LlmError::RequestFailed {
            provider: "claude_cli".to_string(),
            reason: "Failed to capture stdout".to_string(),
        })?;

        let stderr = child.stderr.take().ok_or_else(|| LlmError::RequestFailed {
            provider: "claude_cli".to_string(),
            reason: "Failed to capture stderr".to_string(),
        })?;

        // Spawn stderr reader for debug logging
        let stderr_handle = tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            let mut stderr_output = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "claude_cli", "stderr: {}", line);
                if !stderr_output.is_empty() {
                    stderr_output.push('\n');
                }
                stderr_output.push_str(&line);
            }
            stderr_output
        });

        // Parse NDJSON stdout with timeout
        let timeout = Duration::from_secs(self.config.timeout_secs);
        let parse_result = tokio::time::timeout(timeout, parse_ndjson_stream(stdout)).await;

        let output = match parse_result {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                // Kill the child if parsing failed
                let _ = child.kill().await;
                return Err(e);
            }
            Err(_) => {
                // Timeout
                let _ = child.kill().await;
                return Err(LlmError::RequestFailed {
                    provider: "claude_cli".to_string(),
                    reason: format!("Claude CLI timed out after {}s", self.config.timeout_secs),
                });
            }
        };

        // Wait for the process to exit
        let status = child.wait().await.map_err(|e| LlmError::RequestFailed {
            provider: "claude_cli".to_string(),
            reason: format!("Failed waiting for claude process: {}", e),
        })?;

        let stderr_output = stderr_handle.await.unwrap_or_default();

        if !status.success() {
            let code = status.code().unwrap_or(-1);

            // Check for auth failure
            if stderr_output.contains("not authenticated")
                || stderr_output.contains("auth")
                || code == 1
            {
                return Err(LlmError::AuthFailed {
                    provider: "claude_cli".to_string(),
                });
            }

            return Err(LlmError::RequestFailed {
                provider: "claude_cli".to_string(),
                reason: format!(
                    "Claude CLI exited with code {}{}",
                    code,
                    if stderr_output.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", truncate_str(&stderr_output, 200))
                    }
                ),
            });
        }

        Ok(output)
    }

    /// Execute a request, handling session resume with fallback.
    async fn execute_with_resume(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        tools: &[ToolDefinition],
        thread_id: Option<&str>,
        message_count: usize,
    ) -> Result<CliOutput, LlmError> {
        // Check for existing session to resume
        let resume_info = thread_id.and_then(|tid| self.get_session(tid).map(|s| (tid, s)));

        if let Some((tid, (session_id, _input_count))) = resume_info {
            // Try resuming the session
            match self
                .spawn_claude(prompt, system_prompt, tools, Some(&session_id))
                .await
            {
                Ok(output) => {
                    // Update session state
                    if let Some(ref new_sid) = output.session_id {
                        self.store_session(tid, new_sid.clone(), message_count);
                    }
                    return Ok(output);
                }
                Err(e) => {
                    tracing::warn!(
                        thread_id = tid,
                        "Session resume failed, retrying fresh: {}",
                        e
                    );
                    self.clear_session(tid);
                    // Fall through to fresh request
                }
            }
        }

        // Fresh request (no resume)
        let output = self
            .spawn_claude(prompt, system_prompt, tools, None)
            .await?;

        // Store session if we got one
        if let Some(tid) = thread_id
            && let Some(ref sid) = output.session_id
        {
            self.store_session(tid, sid.clone(), message_count);
        }

        Ok(output)
    }
}

/// Parse NDJSON streaming output from the Claude CLI.
async fn parse_ndjson_stream(stdout: tokio::process::ChildStdout) -> Result<CliOutput, LlmError> {
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();
    let mut output = CliOutput::default();

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let event: ClaudeStreamEvent = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(_) => {
                // Non-JSON line, skip
                tracing::trace!(target: "claude_cli", "Skipping non-JSON line: {}", truncate_str(&line, 100));
                continue;
            }
        };

        // Aggregate usage from top-level or message-level
        aggregate_usage(&mut output, &event);

        match event.event_type.as_str() {
            "system" => {
                if let Some(ref sid) = event.session_id {
                    output.session_id = Some(sid.clone());
                    tracing::debug!(session_id = %sid, "Captured Claude CLI session ID");
                }
            }
            "assistant" => {
                if let Some(ref msg) = event.message
                    && let Some(ref blocks) = msg.content
                {
                    for block in blocks {
                        match block.block_type.as_str() {
                            "text" => {
                                if let Some(ref text) = block.text
                                    && !text.is_empty()
                                {
                                    if !output.text_content.is_empty() {
                                        output.text_content.push('\n');
                                    }
                                    output.text_content.push_str(text);
                                }
                            }
                            "tool_use" => {
                                if let (Some(id), Some(name)) = (&block.id, &block.name) {
                                    output.tool_calls.push(ToolCall {
                                        id: id.clone(),
                                        name: name.clone(),
                                        arguments: block.input.clone().unwrap_or(
                                            serde_json::Value::Object(serde_json::Map::new()),
                                        ),
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            "result" => {
                output.is_error = event.is_error.unwrap_or(false);

                // Extract result text as fallback content
                if let Some(ref result) = event.result
                    && let Some(text) = result.as_str()
                    && !text.is_empty()
                {
                    output.result_text = Some(text.to_string());
                }
            }
            _ => {
                // system, user, or unknown events -- skip
            }
        }
    }

    Ok(output)
}

/// Aggregate usage information from an event into the output.
fn aggregate_usage(output: &mut CliOutput, event: &ClaudeStreamEvent) {
    if let Some(ref usage) = event.usage {
        add_usage(output, usage);
    }
    if let Some(ref msg) = event.message
        && let Some(ref usage) = msg.usage
    {
        add_usage(output, usage);
    }
}

fn add_usage(output: &mut CliOutput, usage: &UsageInfo) {
    output.input_tokens = output.input_tokens.saturating_add(usage.input_tokens);
    output.output_tokens = output.output_tokens.saturating_add(usage.output_tokens);
}

/// Convert IronClaw messages to a single prompt string for the Claude CLI.
///
/// Extracts system messages separately. Combines user/assistant/tool messages
/// into a prompt suitable for `-p`.
fn convert_messages(messages: &[ChatMessage]) -> (Option<String>, String) {
    let mut system_parts = Vec::new();
    let mut prompt_parts = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => {
                system_parts.push(msg.content.clone());
            }
            Role::User => {
                if let Some(ref tool_call_id) = msg.tool_call_id {
                    // This is a tool result formatted as a user message
                    let name = msg.name.as_deref().unwrap_or("tool");
                    prompt_parts.push(format!(
                        "[Tool result for {} ({}): {}]",
                        name, tool_call_id, msg.content
                    ));
                } else {
                    prompt_parts.push(msg.content.clone());
                }
            }
            Role::Assistant => {
                if let Some(ref calls) = msg.tool_calls {
                    let calls_desc: Vec<String> = calls
                        .iter()
                        .map(|c| format!("{}({})", c.name, c.arguments))
                        .collect();
                    if msg.content.is_empty() {
                        prompt_parts
                            .push(format!("[Assistant used tools: {}]", calls_desc.join(", ")));
                    } else {
                        prompt_parts.push(format!(
                            "{}\n[Assistant used tools: {}]",
                            msg.content,
                            calls_desc.join(", ")
                        ));
                    }
                } else if !msg.content.is_empty() {
                    prompt_parts.push(format!("[Assistant: {}]", msg.content));
                }
            }
            Role::Tool => {
                let name = msg.name.as_deref().unwrap_or("tool");
                let call_id = msg.tool_call_id.as_deref().unwrap_or("unknown");
                prompt_parts.push(format!(
                    "[Tool result for {} ({}): {}]",
                    name, call_id, msg.content
                ));
            }
        }
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    let prompt = prompt_parts.join("\n\n");
    (system, prompt)
}

fn truncate_str(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

#[async_trait]
impl LlmProvider for ClaudeCliProvider {
    fn model_name(&self) -> &str {
        &self.config.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        // Claude Max/Pro subscription: no per-token cost
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let (system, prompt) = convert_messages(&request.messages);
        let thread_id = request.metadata.get("thread_id").map(|s| s.as_str());

        let output = self
            .execute_with_resume(
                &prompt,
                system.as_deref(),
                &[],
                thread_id,
                request.messages.len(),
            )
            .await?;

        // Use text content, falling back to result text
        let content = if output.text_content.is_empty() {
            output.result_text.unwrap_or_default()
        } else {
            output.text_content
        };

        if content.is_empty() && !output.is_error {
            tracing::warn!("Claude CLI returned empty content");
        }

        Ok(CompletionResponse {
            content,
            input_tokens: output.input_tokens,
            output_tokens: output.output_tokens,
            finish_reason: if output.is_error {
                FinishReason::Unknown
            } else {
                FinishReason::Stop
            },
            response_id: output.session_id,
        })
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let (system, prompt) = convert_messages(&request.messages);
        let thread_id = request.metadata.get("thread_id").map(|s| s.as_str());

        let output = self
            .execute_with_resume(
                &prompt,
                system.as_deref(),
                &request.tools,
                thread_id,
                request.messages.len(),
            )
            .await?;

        let has_tool_calls = !output.tool_calls.is_empty();

        let content = if output.text_content.is_empty() {
            output.result_text
        } else {
            Some(output.text_content)
        };

        let finish_reason = if output.is_error {
            FinishReason::Unknown
        } else if has_tool_calls {
            FinishReason::ToolUse
        } else {
            FinishReason::Stop
        };

        Ok(ToolCompletionResponse {
            content,
            tool_calls: output.tool_calls,
            input_tokens: output.input_tokens,
            output_tokens: output.output_tokens,
            finish_reason,
            response_id: output.session_id,
        })
    }

    fn active_model_name(&self) -> String {
        self.current_model()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        let mut active = match self.active_model.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *active = model.to_string();
        Ok(())
    }

    fn seed_response_chain(&self, thread_id: &str, response_id: String) {
        self.store_session(thread_id, response_id, 0);
    }

    fn get_response_chain_id(&self, thread_id: &str) -> Option<String> {
        self.get_session(thread_id).map(|(sid, _)| sid)
    }

    /// The CLI does not ignore per-request model overrides. We always use the
    /// active model since the CLI binary only accepts `--model` at launch.
    fn effective_model_name(&self, _requested_model: Option<&str>) -> String {
        self.active_model_name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ClaudeCliConfig {
        ClaudeCliConfig {
            model: "claude-sonnet-4-6-20250520".to_string(),
            max_turns: 1,
            timeout_secs: 300,
            binary_path: "claude".to_string(),
        }
    }

    // --- Message conversion tests ---

    #[test]
    fn test_convert_system_messages_extracted() {
        let messages = vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::user("Hello"),
        ];
        let (system, prompt) = convert_messages(&messages);
        assert_eq!(system.as_deref(), Some("You are a helpful assistant."));
        assert_eq!(prompt, "Hello");
    }

    #[test]
    fn test_convert_multiple_system_messages_concatenated() {
        let messages = vec![
            ChatMessage::system("System 1"),
            ChatMessage::system("System 2"),
            ChatMessage::user("Hello"),
        ];
        let (system, prompt) = convert_messages(&messages);
        assert_eq!(system.as_deref(), Some("System 1\n\nSystem 2"));
        assert_eq!(prompt, "Hello");
    }

    #[test]
    fn test_convert_single_user_message() {
        let messages = vec![ChatMessage::user("What is 2+2?")];
        let (system, prompt) = convert_messages(&messages);
        assert!(system.is_none());
        assert_eq!(prompt, "What is 2+2?");
    }

    #[test]
    fn test_convert_assistant_with_text() {
        let messages = vec![
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi there!"),
            ChatMessage::user("How are you?"),
        ];
        let (system, prompt) = convert_messages(&messages);
        assert!(system.is_none());
        assert!(prompt.contains("Hello"));
        assert!(prompt.contains("[Assistant: Hi there!]"));
        assert!(prompt.contains("How are you?"));
    }

    #[test]
    fn test_convert_assistant_with_tool_calls() {
        let messages = vec![
            ChatMessage::user("List files"),
            ChatMessage::assistant_with_tool_calls(
                None,
                vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::json!({"command": "ls"}),
                }],
            ),
        ];
        let (_system, prompt) = convert_messages(&messages);
        assert!(prompt.contains("[Assistant used tools:"));
        assert!(prompt.contains("shell"));
    }

    #[test]
    fn test_convert_tool_result_messages() {
        let messages = vec![
            ChatMessage::user("List files"),
            ChatMessage::tool_result("call_1", "shell", "file1.txt\nfile2.txt"),
        ];
        let (_system, prompt) = convert_messages(&messages);
        assert!(prompt.contains("[Tool result for shell (call_1):"));
        assert!(prompt.contains("file1.txt"));
    }

    #[test]
    fn test_convert_empty_messages() {
        let messages: Vec<ChatMessage> = vec![];
        let (system, prompt) = convert_messages(&messages);
        assert!(system.is_none());
        assert!(prompt.is_empty());
    }

    #[test]
    fn test_convert_mixed_conversation() {
        let messages = vec![
            ChatMessage::system("Be concise."),
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi!"),
            ChatMessage::user("What time is it?"),
        ];
        let (system, prompt) = convert_messages(&messages);
        assert_eq!(system.as_deref(), Some("Be concise."));
        assert!(prompt.contains("Hello"));
        assert!(prompt.contains("[Assistant: Hi!]"));
        assert!(prompt.contains("What time is it?"));
    }

    // --- NDJSON parsing tests ---

    #[tokio::test]
    async fn test_parse_system_event_captures_session_id() {
        let ndjson = r#"{"type":"system","session_id":"sess-abc-123","subtype":"init"}
{"type":"result","is_error":false,"result":"Done"}"#;
        let output = parse_ndjson_from_str(ndjson).await;
        assert_eq!(output.session_id.as_deref(), Some("sess-abc-123"));
    }

    #[tokio::test]
    async fn test_parse_assistant_text_aggregated() {
        let ndjson = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello "}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"world!"}]}}
{"type":"result","is_error":false}"#;
        let output = parse_ndjson_from_str(ndjson).await;
        assert_eq!(output.text_content, "Hello \nworld!");
    }

    #[tokio::test]
    async fn test_parse_tool_use_blocks() {
        let ndjson = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_01","name":"Bash","input":{"command":"ls"}}]}}
{"type":"result","is_error":false}"#;
        let output = parse_ndjson_from_str(ndjson).await;
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].id, "toolu_01");
        assert_eq!(output.tool_calls[0].name, "Bash");
        assert_eq!(
            output.tool_calls[0].arguments,
            serde_json::json!({"command": "ls"})
        );
    }

    #[tokio::test]
    async fn test_parse_mixed_text_and_tool_use() {
        let ndjson = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Let me check."},{"type":"tool_use","id":"t1","name":"Read","input":{"path":"file.txt"}}]}}
{"type":"result","is_error":false}"#;
        let output = parse_ndjson_from_str(ndjson).await;
        assert_eq!(output.text_content, "Let me check.");
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].name, "Read");
    }

    #[tokio::test]
    async fn test_parse_result_success() {
        let ndjson = r#"{"type":"result","is_error":false,"result":"All done.","num_turns":3}"#;
        let output = parse_ndjson_from_str(ndjson).await;
        assert!(!output.is_error);
        assert_eq!(output.result_text.as_deref(), Some("All done."));
    }

    #[tokio::test]
    async fn test_parse_result_error() {
        let ndjson = r#"{"type":"result","is_error":true,"result":"Something went wrong"}"#;
        let output = parse_ndjson_from_str(ndjson).await;
        assert!(output.is_error);
    }

    #[tokio::test]
    async fn test_parse_usage_info_aggregated() {
        let ndjson = r#"{"type":"assistant","usage":{"input_tokens":100,"output_tokens":50},"message":{"content":[{"type":"text","text":"Hi"}]}}
{"type":"assistant","usage":{"input_tokens":0,"output_tokens":30},"message":{"content":[{"type":"text","text":" there"}]}}
{"type":"result","is_error":false}"#;
        let output = parse_ndjson_from_str(ndjson).await;
        assert_eq!(output.input_tokens, 100);
        assert_eq!(output.output_tokens, 80);
    }

    #[tokio::test]
    async fn test_parse_non_json_lines_skipped() {
        let ndjson = "not json at all\n{\"type\":\"result\",\"is_error\":false,\"result\":\"ok\"}\nmore garbage";
        let output = parse_ndjson_from_str(ndjson).await;
        assert!(!output.is_error);
        assert_eq!(output.result_text.as_deref(), Some("ok"));
    }

    #[tokio::test]
    async fn test_parse_empty_stream() {
        let ndjson = "";
        let output = parse_ndjson_from_str(ndjson).await;
        assert!(output.session_id.is_none());
        assert!(output.text_content.is_empty());
        assert!(output.tool_calls.is_empty());
    }

    // --- Session tracking tests ---

    #[test]
    fn test_session_store_and_get() {
        let provider = ClaudeCliProvider::new(test_config());
        provider.store_session("thread-1", "sess-abc".to_string(), 5);
        let (sid, count) = provider.get_session("thread-1").unwrap();
        assert_eq!(sid, "sess-abc");
        assert_eq!(count, 5);
    }

    #[test]
    fn test_session_clear() {
        let provider = ClaudeCliProvider::new(test_config());
        provider.store_session("thread-1", "sess-abc".to_string(), 5);
        provider.clear_session("thread-1");
        assert!(provider.get_session("thread-1").is_none());
    }

    #[test]
    fn test_session_overwrite() {
        let provider = ClaudeCliProvider::new(test_config());
        provider.store_session("thread-1", "sess-old".to_string(), 3);
        provider.store_session("thread-1", "sess-new".to_string(), 7);
        let (sid, count) = provider.get_session("thread-1").unwrap();
        assert_eq!(sid, "sess-new");
        assert_eq!(count, 7);
    }

    #[test]
    fn test_session_independent_threads() {
        let provider = ClaudeCliProvider::new(test_config());
        provider.store_session("thread-1", "sess-1".to_string(), 1);
        provider.store_session("thread-2", "sess-2".to_string(), 2);
        assert_eq!(provider.get_session("thread-1").unwrap().0, "sess-1");
        assert_eq!(provider.get_session("thread-2").unwrap().0, "sess-2");
    }

    #[test]
    fn test_session_missing_thread() {
        let provider = ClaudeCliProvider::new(test_config());
        assert!(provider.get_session("nonexistent").is_none());
    }

    // --- Provider trait tests ---

    #[test]
    fn test_cost_per_token_is_zero() {
        let provider = ClaudeCliProvider::new(test_config());
        let (input, output) = provider.cost_per_token();
        assert_eq!(input, Decimal::ZERO);
        assert_eq!(output, Decimal::ZERO);
    }

    #[test]
    fn test_model_name() {
        let provider = ClaudeCliProvider::new(test_config());
        assert_eq!(provider.model_name(), "claude-sonnet-4-6-20250520");
    }

    #[test]
    fn test_set_model() {
        let provider = ClaudeCliProvider::new(test_config());
        provider.set_model("claude-opus-4-20250514").unwrap();
        assert_eq!(provider.active_model_name(), "claude-opus-4-20250514");
    }

    #[test]
    fn test_seed_and_get_response_chain() {
        let provider = ClaudeCliProvider::new(test_config());
        provider.seed_response_chain("thread-1", "sess-123".to_string());
        assert_eq!(
            provider.get_response_chain_id("thread-1").as_deref(),
            Some("sess-123")
        );
    }

    #[test]
    fn test_effective_model_ignores_override() {
        let provider = ClaudeCliProvider::new(test_config());
        // CLI always uses its active model, ignoring per-request overrides
        assert_eq!(
            provider.effective_model_name(Some("gpt-4o")),
            "claude-sonnet-4-6-20250520"
        );
    }

    // --- Test helpers ---

    /// Parse NDJSON from a string (test helper).
    async fn parse_ndjson_from_str(ndjson: &str) -> CliOutput {
        let bytes = ndjson.as_bytes();
        let cursor = std::io::Cursor::new(bytes.to_vec());
        let reader = BufReader::new(cursor);
        let mut lines = reader.lines();
        let mut output = CliOutput::default();

        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }

            let event: ClaudeStreamEvent = match serde_json::from_str(&line) {
                Ok(e) => e,
                Err(_) => continue,
            };

            aggregate_usage(&mut output, &event);

            match event.event_type.as_str() {
                "system" => {
                    if let Some(ref sid) = event.session_id {
                        output.session_id = Some(sid.clone());
                    }
                }
                "assistant" => {
                    if let Some(ref msg) = event.message
                        && let Some(ref blocks) = msg.content
                    {
                        for block in blocks {
                            match block.block_type.as_str() {
                                "text" => {
                                    if let Some(ref text) = block.text
                                        && !text.is_empty()
                                    {
                                        if !output.text_content.is_empty() {
                                            output.text_content.push('\n');
                                        }
                                        output.text_content.push_str(text);
                                    }
                                }
                                "tool_use" => {
                                    if let (Some(id), Some(name)) = (&block.id, &block.name) {
                                        output.tool_calls.push(ToolCall {
                                            id: id.clone(),
                                            name: name.clone(),
                                            arguments: block.input.clone().unwrap_or(
                                                serde_json::Value::Object(
                                                    serde_json::Map::new(),
                                                ),
                                            ),
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                "result" => {
                    output.is_error = event.is_error.unwrap_or(false);
                    if let Some(ref result) = event.result
                        && let Some(text) = result.as_str()
                        && !text.is_empty()
                    {
                        output.result_text = Some(text.to_string());
                    }
                }
                _ => {}
            }
        }

        output
    }
}
