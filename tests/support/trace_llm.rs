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

// ---------------------------------------------------------------------------
// Trace types
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
/// (no `"turns"`) are deserialized as a single turn with a placeholder user message.
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

/// A memory document captured at recording start.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySnapshotEntry {
    pub path: String,
    pub content: String,
}

/// A recorded HTTP request/response pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpExchange {
    pub request: HttpExchangeRequest,
    pub response: HttpExchangeResponse,
}

/// The request side of an HTTP exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpExchangeRequest {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub body: Option<String>,
}

/// The response side of an HTTP exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpExchangeResponse {
    pub status: u16,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// Recorded tool result for regression checking during replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedToolResult {
    pub tool_call_id: String,
    pub name: String,
    pub content: String,
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
            // Filter out user_input steps for the turn's playable steps.
            let playable: Vec<TraceStep> = raw
                .steps
                .into_iter()
                .filter(|s| !matches!(s.response, TraceResponse::UserInput { .. }))
                .collect();
            vec![TraceTurn {
                user_input: "(test input)".to_string(),
                steps: playable,
                expects: TraceExpects::default(),
            }]
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

/// A single step in a trace, pairing an optional request hint with a response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStep {
    /// Optional hint for soft-validating the incoming request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_hint: Option<RequestHint>,
    /// The canned response to return for this step.
    pub response: TraceResponse,
    /// Tool results that appeared since the previous step (for replay verification).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_tool_results: Vec<ExpectedToolResult>,
}

/// Hints for soft-validating a request against expectations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestHint {
    /// If set, the last user message should contain this substring.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_user_message_contains: Option<String>,
    /// If set, the message list should have at least this many messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_message_count: Option<usize>,
}

/// A canned response -- text, tool calls, or a user_input marker.
///
/// `UserInput` steps are metadata markers emitted by `RecordingLlm` to record
/// what the user said. They do **not** correspond to LLM calls and are filtered
/// out before replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TraceResponse {
    Text {
        content: String,
        input_tokens: u32,
        output_tokens: u32,
    },
    ToolCalls {
        tool_calls: Vec<TraceToolCall>,
        input_tokens: u32,
        output_tokens: u32,
    },
    /// Marker for a user message (recording only — skipped during replay).
    UserInput { content: String },
}

/// A single tool call in a trace response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

// ---------------------------------------------------------------------------
// TraceLlm provider
// ---------------------------------------------------------------------------

/// An `LlmProvider` that replays canned responses from a trace.
///
/// Steps from all turns are flattened into a single sequence at construction
/// time. The provider advances through them linearly regardless of turn
/// boundaries.
pub struct TraceLlm {
    model_name: String,
    steps: Vec<TraceStep>,
    index: AtomicUsize,
    hint_mismatches: AtomicUsize,
    captured_requests: Mutex<Vec<Vec<ChatMessage>>>,
}

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
    fn next_step(&self, messages: &[ChatMessage]) -> Result<TraceStep, LlmError> {
        // Capture the request messages.
        self.captured_requests
            .lock()
            .unwrap()
            .push(messages.to_vec());

        let idx = self.index.fetch_add(1, Ordering::Relaxed);
        let step = self
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
        let step = self.next_step(&request.messages)?;
        match step.response {
            TraceResponse::Text {
                content,
                input_tokens,
                output_tokens,
            } => Ok(CompletionResponse {
                content,
                input_tokens,
                output_tokens,
                finish_reason: FinishReason::Stop,
            }),
            TraceResponse::ToolCalls { .. } => Err(LlmError::RequestFailed {
                provider: self.model_name.clone(),
                reason: "TraceLlm::complete() called but current step is a tool_calls response; \
                         use complete_with_tools() instead"
                    .to_string(),
            }),
            TraceResponse::UserInput { .. } => Err(LlmError::RequestFailed {
                provider: self.model_name.clone(),
                reason: "TraceLlm::complete() encountered a user_input step; \
                         these should have been filtered out during construction"
                    .to_string(),
            }),
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
                    })
                    .collect();
                Ok(ToolCompletionResponse {
                    content: None,
                    tool_calls: calls,
                    input_tokens,
                    output_tokens,
                    finish_reason: FinishReason::ToolUse,
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
