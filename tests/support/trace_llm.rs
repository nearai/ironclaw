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
