//! TraceLlm -- a replay-based LLM provider for E2E testing.
//!
//! Replays canned responses from a JSON trace, advancing through steps
//! sequentially. Supports both text and tool-call responses with optional
//! request-hint validation.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use ironclaw::error::LlmError;
use ironclaw::llm::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse,
};

// Re-export shared types from recording module so existing test code can
// still import them from here.
// Re-export all shared types so downstream test files can import from here.
#[allow(unused_imports)]
pub use ironclaw::llm::recording::{
    ExpectedToolResult, HttpExchange, HttpExchangeRequest, HttpExchangeResponse,
    MemorySnapshotEntry, RequestHint, TraceResponse, TraceStep, TraceToolCall,
};

// ---------------------------------------------------------------------------
// Trace types (test-only wrappers around shared recording types)
// ---------------------------------------------------------------------------

/// A single turn in a trace: one user message and the LLM response steps that follow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceTurn {
    pub user_input: String,
    pub steps: Vec<TraceStep>,
    /// Declarative expectations for this turn (optional).
    #[serde(default, skip_serializing_if = "TraceExpects::is_empty")]
    pub expects: TraceExpects,
}

/// A complete LLM trace: a model name and an ordered list of turns.
///
/// Each turn pairs a user message with the LLM response steps that follow it.
/// For JSON backward compatibility, traces with a flat top-level `"steps"` array
/// (no `"turns"`) are deserialized into turns by splitting at `UserInput` boundaries.
///
/// Recorded traces (from `RecordingLlm`) may also include `memory_snapshot`,
/// `http_exchanges`, and `user_input` response steps.
#[derive(Debug, Clone, Serialize)]
pub struct LlmTrace {
    pub model_name: String,
    pub turns: Vec<TraceTurn>,
    /// Workspace memory documents captured before the recording session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_snapshot: Vec<MemorySnapshotEntry>,
    /// HTTP exchanges recorded during the session, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub http_exchanges: Vec<HttpExchange>,
    /// Declarative expectations for the whole trace (optional).
    #[serde(default, skip_serializing_if = "TraceExpects::is_empty")]
    pub expects: TraceExpects,
    /// Raw steps before turn conversion (populated only for recorded traces).
    /// Used by `playable_steps()` for recorded-format inspection.
    #[serde(skip)]
    #[allow(dead_code)]
    pub steps: Vec<TraceStep>,
}

/// Declarative expectations for a trace or turn.
///
/// All fields are optional and default to empty/None, so traces without
/// `expects` work unchanged (backward compatible).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TraceExpects {
    /// Each string must appear in the response (case-insensitive).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub response_contains: Vec<String>,
    /// None of these may appear in the response (case-insensitive).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub response_not_contains: Vec<String>,
    /// Regex that must match the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_matches: Option<String>,
    /// Each tool name must appear in started calls.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools_used: Vec<String>,
    /// None of these tool names may appear.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools_not_used: Vec<String>,
    /// If true, all tools must succeed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_tools_succeeded: Option<bool>,
    /// Upper bound on tool call count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_calls: Option<usize>,
    /// Minimum response count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_responses: Option<usize>,
    /// Tool result preview must contain substring (tool_name -> substring).
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub tool_results_contain: std::collections::HashMap<String, String>,
    /// Tools must have been called in this relative order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools_order: Vec<String>,
}

impl TraceExpects {
    /// Returns true if no expectations are set.
    pub fn is_empty(&self) -> bool {
        self.response_contains.is_empty()
            && self.response_not_contains.is_empty()
            && self.response_matches.is_none()
            && self.tools_used.is_empty()
            && self.tools_not_used.is_empty()
            && self.all_tools_succeeded.is_none()
            && self.max_tool_calls.is_none()
            && self.min_responses.is_none()
            && self.tool_results_contain.is_empty()
            && self.tools_order.is_empty()
    }
}

/// Raw deserialization helper -- accepts either `turns` or flat `steps`.
#[derive(Deserialize)]
struct RawLlmTrace {
    model_name: String,
    #[serde(default)]
    steps: Vec<TraceStep>,
    #[serde(default)]
    turns: Vec<TraceTurn>,
    #[serde(default)]
    memory_snapshot: Vec<MemorySnapshotEntry>,
    #[serde(default)]
    http_exchanges: Vec<HttpExchange>,
    #[serde(default)]
    expects: TraceExpects,
}

impl<'de> Deserialize<'de> for LlmTrace {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawLlmTrace::deserialize(deserializer)?;
        // Keep the raw steps for `playable_steps()` inspection.
        let raw_steps = raw.steps.clone();
        let turns = if !raw.turns.is_empty() {
            raw.turns
        } else if !raw.steps.is_empty() {
            // Split flat steps at UserInput boundaries into turns.
            let mut turns = Vec::new();
            let mut current_input = "(test input)".to_string();
            let mut current_steps: Vec<TraceStep> = Vec::new();

            for step in raw.steps {
                if let TraceResponse::UserInput { ref content } = step.response {
                    // Flush accumulated steps as a turn (if any).
                    if !current_steps.is_empty() {
                        turns.push(TraceTurn {
                            user_input: current_input.clone(),
                            steps: std::mem::take(&mut current_steps),
                            expects: TraceExpects::default(),
                        });
                    }
                    current_input = content.clone();
                } else {
                    current_steps.push(step);
                }
            }

            // Flush remaining steps.
            if !current_steps.is_empty() {
                turns.push(TraceTurn {
                    user_input: current_input,
                    steps: current_steps,
                    expects: TraceExpects::default(),
                });
            }

            turns
        } else {
            vec![]
        };
        Ok(LlmTrace {
            model_name: raw.model_name,
            turns,
            memory_snapshot: raw.memory_snapshot,
            http_exchanges: raw.http_exchanges,
            expects: raw.expects,
            steps: raw_steps,
        })
    }
}

#[allow(dead_code)]
impl LlmTrace {
    /// Create a trace from turns.
    pub fn new(model_name: impl Into<String>, turns: Vec<TraceTurn>) -> Self {
        Self {
            model_name: model_name.into(),
            turns,
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            expects: TraceExpects::default(),
            steps: Vec::new(),
        }
    }

    /// Convenience: create a single-turn trace (for simple tests).
    pub fn single_turn(
        model_name: impl Into<String>,
        user_input: impl Into<String>,
        steps: Vec<TraceStep>,
    ) -> Self {
        Self {
            model_name: model_name.into(),
            turns: vec![TraceTurn {
                user_input: user_input.into(),
                steps,
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            expects: TraceExpects::default(),
            steps: Vec::new(),
        }
    }

    /// Load a trace from a JSON file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let contents = std::fs::read_to_string(path)?;
        let trace: Self = serde_json::from_str(&contents)?;
        Ok(trace)
    }

    /// Return only the playable steps from the raw steps (text + tool_calls),
    /// skipping `user_input` markers. Only meaningful for recorded traces that
    /// were deserialized from a flat `steps` array.
    #[allow(dead_code)]
    pub fn playable_steps(&self) -> Vec<&TraceStep> {
        self.steps
            .iter()
            .filter(|s| !matches!(s.response, TraceResponse::UserInput { .. }))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// TraceLlm provider
// ---------------------------------------------------------------------------

/// An `LlmProvider` that replays canned responses from a trace.
///
/// Steps from all turns are flattened into a single sequence at construction
/// time. The provider advances through them linearly regardless of turn
/// boundaries.
///
/// **Concurrency assumption:** Uses `AtomicUsize` for step indexing, so
/// concurrent calls to `complete`/`complete_with_tools` may consume steps
/// in non-deterministic order. Current tests are single-threaded per rig;
/// if parallel tool execution is ever enabled, steps may interleave.
pub struct TraceLlm {
    model_name: String,
    steps: Vec<TraceStep>,
    index: AtomicUsize,
    hint_mismatches: AtomicUsize,
    captured_requests: Mutex<Vec<Vec<ChatMessage>>>,
}

#[allow(dead_code)]
impl TraceLlm {
    /// Create from an in-memory trace.
    pub fn from_trace(trace: LlmTrace) -> Self {
        let steps: Vec<TraceStep> = trace.turns.into_iter().flat_map(|t| t.steps).collect();
        Self {
            model_name: trace.model_name,
            steps,
            index: AtomicUsize::new(0),
            hint_mismatches: AtomicUsize::new(0),
            captured_requests: Mutex::new(Vec::new()),
        }
    }

    /// Load from a JSON file and create the provider.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let trace = LlmTrace::from_file(path)?;
        Ok(Self::from_trace(trace))
    }

    /// Number of calls made so far.
    pub fn calls(&self) -> usize {
        self.index.load(Ordering::Relaxed)
    }

    /// Number of request-hint mismatches observed (warnings only).
    pub fn hint_mismatches(&self) -> usize {
        self.hint_mismatches.load(Ordering::Relaxed)
    }

    /// Clone of all captured request message lists.
    pub fn captured_requests(&self) -> Vec<Vec<ChatMessage>> {
        self.captured_requests.lock().unwrap().clone()
    }

    // -- internal helpers ---------------------------------------------------

    /// Advance the step index and return the current step, or an error if exhausted.
    ///
    /// Before returning, applies template substitution on tool_call arguments:
    /// `{{call_id.json_path}}` is replaced with the value extracted from the
    /// tool result message whose `tool_call_id` matches `call_id`. The
    /// `json_path` is a dot-separated path into the JSON content of that tool
    /// result (e.g., `{{call_cj_1.job_id}}` extracts `.job_id` from the result
    /// of tool call `call_cj_1`).
    fn next_step(&self, messages: &[ChatMessage]) -> Result<TraceStep, LlmError> {
        // Capture the request messages.
        self.captured_requests
            .lock()
            .unwrap()
            .push(messages.to_vec());

        let idx = self.index.fetch_add(1, Ordering::Relaxed);
        let mut step = self
            .steps
            .get(idx)
            .ok_or_else(|| LlmError::RequestFailed {
                provider: self.model_name.clone(),
                reason: format!(
                    "TraceLlm exhausted: called {} times but only {} steps",
                    idx + 1,
                    self.steps.len()
                ),
            })?
            .clone();

        // Soft-validate request hints.
        if let Some(ref hint) = step.request_hint {
            self.validate_hint(hint, messages);
        }

        // Apply template substitution on tool_call arguments.
        if let TraceResponse::ToolCalls {
            ref mut tool_calls, ..
        } = step.response
        {
            let vars = Self::extract_tool_result_vars(messages);
            if !vars.is_empty() {
                for tc in tool_calls.iter_mut() {
                    Self::substitute_templates(&mut tc.arguments, &vars);
                }
            }
        }

        Ok(step)
    }

    fn validate_hint(&self, hint: &RequestHint, messages: &[ChatMessage]) {
        if let Some(ref expected_substr) = hint.last_user_message_contains {
            let last_user = messages.iter().rev().find(|m| matches!(m.role, Role::User));
            let matched = last_user
                .map(|m| m.content.contains(expected_substr.as_str()))
                .unwrap_or(false);
            if !matched {
                self.hint_mismatches.fetch_add(1, Ordering::Relaxed);
                eprintln!(
                    "[TraceLlm WARN] Request hint mismatch: expected last user message to contain {:?}, \
                     got {:?}",
                    expected_substr,
                    last_user.map(|m| &m.content),
                );
            }
        }

        if let Some(min_count) = hint.min_message_count
            && messages.len() < min_count
        {
            self.hint_mismatches.fetch_add(1, Ordering::Relaxed);
            eprintln!(
                "[TraceLlm WARN] Request hint mismatch: expected >= {} messages, got {}",
                min_count,
                messages.len(),
            );
        }
    }

    /// Build a map of `"call_id.json_path" -> resolved_value` from tool result
    /// messages in the conversation. Each `Role::Tool` message with a
    /// `tool_call_id` has its content parsed as JSON; all top-level
    /// string/number/bool values are indexed so that `{{call_id.key}}` can be
    /// resolved.
    ///
    /// Tool results may be wrapped in `<tool_output>` XML tags by the safety
    /// layer, so we strip those before parsing.
    fn extract_tool_result_vars(
        messages: &[ChatMessage],
    ) -> std::collections::HashMap<String, String> {
        let mut vars = std::collections::HashMap::new();
        for msg in messages {
            if msg.role != Role::Tool {
                continue;
            }
            let call_id = match &msg.tool_call_id {
                Some(id) => id,
                None => continue,
            };
            // Strip <tool_output ...>...</tool_output> wrapper if present.
            let content = Self::unwrap_tool_output(&msg.content);
            // Try parsing the content as JSON.
            let json: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(obj) = json.as_object() {
                for (key, val) in obj {
                    let str_val = match val {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        _ => continue,
                    };
                    vars.insert(format!("{call_id}.{key}"), str_val);
                }
            }
        }
        vars
    }

    /// Strip `<tool_output name="...">...\n</tool_output>` wrapper from
    /// safety-layer output and reverse the targeted `</tool_output` escape.
    fn unwrap_tool_output(content: &str) -> std::borrow::Cow<'_, str> {
        if let Some(body) = ironclaw_safety::SafetyLayer::unwrap_tool_output(content) {
            return std::borrow::Cow::Owned(body);
        }
        std::borrow::Cow::Borrowed(content)
    }

    /// Walk a JSON value and replace any string matching `{{call_id.path}}`
    /// with the resolved value from the vars map. Operates in-place.
    fn substitute_templates(
        value: &mut serde_json::Value,
        vars: &std::collections::HashMap<String, String>,
    ) {
        match value {
            serde_json::Value::String(s) => {
                // Full-value replacement: if the entire string is `{{...}}`,
                // replace the whole value (preserving type if possible).
                if s.starts_with("{{") && s.ends_with("}}") && s.matches("{{").count() == 1 {
                    let key = s[2..s.len() - 2].trim();
                    if let Some(resolved) = vars.get(key) {
                        *s = resolved.clone();
                        return;
                    }
                }
                // Inline replacement: replace all `{{...}}` occurrences within the string.
                let mut result = s.clone();
                while let Some(start) = result.find("{{") {
                    if let Some(end) = result[start..].find("}}") {
                        let end = start + end + 2;
                        let key = result[start + 2..end - 2].trim();
                        if let Some(resolved) = vars.get(key) {
                            result = format!("{}{}{}", &result[..start], resolved, &result[end..]);
                        } else {
                            // Unresolved template — leave as-is and stop to avoid infinite loop.
                            break;
                        }
                    } else {
                        break;
                    }
                }
                *s = result;
            }
            serde_json::Value::Object(map) => {
                for val in map.values_mut() {
                    Self::substitute_templates(val, vars);
                }
            }
            serde_json::Value::Array(arr) => {
                for val in arr.iter_mut() {
                    Self::substitute_templates(val, vars);
                }
            }
            _ => {}
        }
    }
}

#[async_trait]
impl LlmProvider for TraceLlm {
    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        // complete() is called when Reasoning has force_text=true (no tools
        // available). Skip any remaining ToolCalls steps in the trace and
        // return the next Text step, since in real usage the LLM would
        // produce text when no tools are offered.
        loop {
            let step = self.next_step(&request.messages)?;
            match step.response {
                TraceResponse::Text {
                    content,
                    input_tokens,
                    output_tokens,
                } => {
                    return Ok(CompletionResponse {
                        content,
                        input_tokens,
                        output_tokens,
                        finish_reason: FinishReason::Stop,
                        cache_read_input_tokens: 0,
                        cache_creation_input_tokens: 0,
                    });
                }
                TraceResponse::ToolCalls { .. } => {
                    // Skip tool_calls steps — complete() is called in
                    // force_text mode so the LLM can't use tools anyway.
                    continue;
                }
                TraceResponse::UserInput { .. } => {
                    return Err(LlmError::RequestFailed {
                        provider: self.model_name.clone(),
                        reason: "TraceLlm::complete() encountered a user_input step; \
                                 these should have been filtered out during construction"
                            .to_string(),
                    });
                }
            }
        }
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let step = self.next_step(&request.messages)?;
        match step.response {
            TraceResponse::Text {
                content,
                input_tokens,
                output_tokens,
            } => Ok(ToolCompletionResponse {
                content: Some(content),
                tool_calls: Vec::new(),
                input_tokens,
                output_tokens,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            }),
            TraceResponse::ToolCalls {
                tool_calls,
                input_tokens,
                output_tokens,
            } => {
                let calls: Vec<ToolCall> = tool_calls
                    .into_iter()
                    .map(|tc| ToolCall {
                        id: tc.id,
                        name: tc.name,
                        arguments: tc.arguments,
                        reasoning: None,
                    })
                    .collect();
                Ok(ToolCompletionResponse {
                    content: None,
                    tool_calls: calls,
                    input_tokens,
                    output_tokens,
                    finish_reason: FinishReason::ToolUse,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }
            TraceResponse::UserInput { .. } => Err(LlmError::RequestFailed {
                provider: self.model_name.clone(),
                reason: "TraceLlm::complete_with_tools() encountered a user_input step; \
                         these should have been filtered out during construction"
                    .to_string(),
            }),
        }
    }
}
