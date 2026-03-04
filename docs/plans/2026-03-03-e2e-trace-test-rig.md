# E2E Trace Test Rig Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build an in-process E2E test harness that runs the real agent loop with recorded/replayed LLM traces, validating full message-to-response trajectories including tool execution.

**Architecture:** A `TraceLlm` provider replays canned LLM responses (including tool calls) from JSON fixture files. A `TestChannel` injects messages and captures responses/status events. A `TestRig` builder wires them together with a real `Agent`, real tools, and an isolated libSQL database. Integration tests validate complete flows deterministically in CI.

**Tech Stack:** Rust, tokio, serde_json, libSQL (via existing TestHarnessBuilder patterns), existing ironclaw Agent/Channel/Tool infrastructure.

**Reference:** Based on [Illia's implementation plan](https://github.com/nearai/ironclaw/issues/467) and patterns from [nearai/benchmarks](https://github.com/nearai/benchmarks).

---

## Task 1: TraceLlm Provider

Build an `LlmProvider` implementation that replays canned responses from a JSON trace file. This is the foundation -- everything else depends on it.

**Files:**
- Create: `tests/support/mod.rs`
- Create: `tests/support/trace_llm.rs`

**Step 1: Write the failing test**

Create `tests/support/mod.rs`:
```rust
pub mod trace_llm;
```

Create `tests/support/trace_llm.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_trace_llm_replays_text_response() {
        let trace = LlmTrace {
            model_name: "test-model".to_string(),
            steps: vec![TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: "Hello, world!".to_string(),
                    input_tokens: 100,
                    output_tokens: 20,
                },
            }],
        };

        let llm = TraceLlm::from_trace(trace);

        let request = ToolCompletionRequest::new(
            vec![ChatMessage::user("Hi")],
            vec![],
        );
        let response = llm.complete_with_tools(request).await.unwrap();

        assert_eq!(response.content, Some("Hello, world!".to_string()));
        assert!(response.tool_calls.is_empty());
        assert_eq!(response.input_tokens, 100);
        assert_eq!(response.output_tokens, 20);
        assert_eq!(llm.calls(), 1);
    }

    #[tokio::test]
    async fn test_trace_llm_replays_tool_calls() {
        let trace = LlmTrace {
            model_name: "test-model".to_string(),
            steps: vec![TraceStep {
                request_hint: None,
                response: TraceResponse::ToolCalls {
                    tool_calls: vec![TraceToolCall {
                        id: "call_1".to_string(),
                        name: "time".to_string(),
                        arguments: serde_json::json!({}),
                    }],
                    input_tokens: 100,
                    output_tokens: 30,
                },
            }],
        };

        let llm = TraceLlm::from_trace(trace);

        let request = ToolCompletionRequest::new(
            vec![ChatMessage::user("What time is it?")],
            vec![],
        );
        let response = llm.complete_with_tools(request).await.unwrap();

        assert!(response.content.is_none());
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "time");
    }

    #[tokio::test]
    async fn test_trace_llm_advances_through_steps() {
        let trace = LlmTrace {
            model_name: "test-model".to_string(),
            steps: vec![
                TraceStep {
                    request_hint: None,
                    response: TraceResponse::ToolCalls {
                        tool_calls: vec![TraceToolCall {
                            id: "call_1".to_string(),
                            name: "time".to_string(),
                            arguments: serde_json::json!({}),
                        }],
                        input_tokens: 100,
                        output_tokens: 30,
                    },
                },
                TraceStep {
                    request_hint: None,
                    response: TraceResponse::Text {
                        content: "The time is 3pm.".to_string(),
                        input_tokens: 200,
                        output_tokens: 10,
                    },
                },
            ],
        };

        let llm = TraceLlm::from_trace(trace);

        // First call: tool call
        let r1 = llm
            .complete_with_tools(ToolCompletionRequest::new(
                vec![ChatMessage::user("time?")],
                vec![],
            ))
            .await
            .unwrap();
        assert_eq!(r1.tool_calls.len(), 1);

        // Second call: text response
        let r2 = llm
            .complete_with_tools(ToolCompletionRequest::new(
                vec![ChatMessage::user("time?")],
                vec![],
            ))
            .await
            .unwrap();
        assert_eq!(r2.content, Some("The time is 3pm.".to_string()));
        assert!(r2.tool_calls.is_empty());

        assert_eq!(llm.calls(), 2);
    }

    #[tokio::test]
    async fn test_trace_llm_errors_when_exhausted() {
        let trace = LlmTrace {
            model_name: "test-model".to_string(),
            steps: vec![TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: "done".to_string(),
                    input_tokens: 10,
                    output_tokens: 5,
                },
            }],
        };

        let llm = TraceLlm::from_trace(trace);

        // First call succeeds
        llm.complete_with_tools(ToolCompletionRequest::new(vec![], vec![]))
            .await
            .unwrap();

        // Second call should error
        let result = llm
            .complete_with_tools(ToolCompletionRequest::new(vec![], vec![]))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_trace_llm_validates_request_hints() {
        let trace = LlmTrace {
            model_name: "test-model".to_string(),
            steps: vec![TraceStep {
                request_hint: Some(RequestHint {
                    last_user_message_contains: Some("schedule".to_string()),
                    min_message_count: None,
                }),
                response: TraceResponse::Text {
                    content: "ok".to_string(),
                    input_tokens: 10,
                    output_tokens: 5,
                },
            }],
        };

        let llm = TraceLlm::from_trace(trace);

        // Matching hint passes
        let result = llm
            .complete_with_tools(ToolCompletionRequest::new(
                vec![ChatMessage::user("please schedule a meeting")],
                vec![],
            ))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_trace_llm_hint_mismatch_warns_but_continues() {
        let trace = LlmTrace {
            model_name: "test-model".to_string(),
            steps: vec![TraceStep {
                request_hint: Some(RequestHint {
                    last_user_message_contains: Some("schedule".to_string()),
                    min_message_count: None,
                }),
                response: TraceResponse::Text {
                    content: "ok".to_string(),
                    input_tokens: 10,
                    output_tokens: 5,
                },
            }],
        };

        let llm = TraceLlm::from_trace(trace);

        // Mismatching hint: still returns response (non-brittle)
        // but records the mismatch
        let result = llm
            .complete_with_tools(ToolCompletionRequest::new(
                vec![ChatMessage::user("what is the weather")],
                vec![],
            ))
            .await;
        assert!(result.is_ok());
        assert_eq!(llm.hint_mismatches(), 1);
    }

    #[tokio::test]
    async fn test_trace_llm_from_json_file() {
        // Uses the fixture file created in Step 3
        let trace = LlmTrace::from_file("tests/fixtures/llm_traces/simple_text.json")
            .expect("load trace");
        assert_eq!(trace.model_name, "test-model");
        assert_eq!(trace.steps.len(), 1);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --test support -- 2>&1 || true`

Expected: Compilation error -- `TraceLlm`, `LlmTrace`, etc. not defined.

**Step 3: Write the implementation**

In `tests/support/trace_llm.rs`, above the `#[cfg(test)]` block:

```rust
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use ironclaw::error::LlmError;
use ironclaw::llm::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider,
    ToolCall, ToolCompletionRequest, ToolCompletionResponse,
};

/// A single tool call in a trace step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// The canned response for a trace step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TraceResponse {
    #[serde(rename = "text")]
    Text {
        content: String,
        input_tokens: u32,
        output_tokens: u32,
    },
    #[serde(rename = "tool_calls")]
    ToolCalls {
        tool_calls: Vec<TraceToolCall>,
        input_tokens: u32,
        output_tokens: u32,
    },
}

/// Soft validation hints for a request (non-brittle).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RequestHint {
    /// If set, the last user message must contain this substring.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_user_message_contains: Option<String>,
    /// If set, the request must have at least this many messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_message_count: Option<usize>,
}

/// One step in an LLM trace (request hint + canned response).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStep {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_hint: Option<RequestHint>,
    pub response: TraceResponse,
}

/// A complete LLM trace: model name + ordered steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmTrace {
    pub model_name: String,
    pub steps: Vec<TraceStep>,
}

impl LlmTrace {
    /// Load a trace from a JSON file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let trace: Self = serde_json::from_str(&content)?;
        Ok(trace)
    }
}

/// An LlmProvider that replays canned responses from a trace.
///
/// Advances through steps sequentially. Returns an error if steps are
/// exhausted. Validates request hints (soft: warns but continues on mismatch).
pub struct TraceLlm {
    trace: LlmTrace,
    step_index: AtomicUsize,
    call_count: AtomicU32,
    hint_mismatches: AtomicU32,
    /// Captured requests for post-hoc inspection.
    captured_requests: Mutex<Vec<Vec<ChatMessage>>>,
}

impl TraceLlm {
    /// Create from an in-memory trace.
    pub fn from_trace(trace: LlmTrace) -> Self {
        Self {
            trace,
            step_index: AtomicUsize::new(0),
            call_count: AtomicU32::new(0),
            hint_mismatches: AtomicU32::new(0),
            captured_requests: Mutex::new(Vec::new()),
        }
    }

    /// Load from a JSON file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let trace = LlmTrace::from_file(path)?;
        Ok(Self::from_trace(trace))
    }

    /// Number of LLM calls made.
    pub fn calls(&self) -> u32 {
        self.call_count.load(Ordering::Relaxed)
    }

    /// Number of request hint mismatches encountered.
    pub fn hint_mismatches(&self) -> u32 {
        self.hint_mismatches.load(Ordering::Relaxed)
    }

    /// Get captured request messages for inspection.
    pub fn captured_requests(&self) -> Vec<Vec<ChatMessage>> {
        self.captured_requests.lock().unwrap().clone()
    }

    fn next_step(&self) -> Result<TraceStep, LlmError> {
        let idx = self.step_index.fetch_add(1, Ordering::Relaxed);
        if idx >= self.trace.steps.len() {
            return Err(LlmError::RequestFailed {
                provider: self.trace.model_name.clone(),
                reason: format!(
                    "TraceLlm exhausted: {} steps consumed, no more available",
                    self.trace.steps.len()
                ),
            });
        }
        Ok(self.trace.steps[idx].clone())
    }

    fn validate_hint(&self, hint: &RequestHint, messages: &[ChatMessage]) {
        let mut mismatch = false;

        if let Some(ref contains) = hint.last_user_message_contains {
            let last_user = messages
                .iter()
                .rev()
                .find(|m| matches!(m.role, ironclaw::llm::Role::User));
            if let Some(msg) = last_user {
                if !msg.content.to_lowercase().contains(&contains.to_lowercase()) {
                    tracing::warn!(
                        "TraceLlm hint mismatch: expected last user message to contain '{}', got '{}'",
                        contains,
                        &msg.content[..msg.content.len().min(100)]
                    );
                    mismatch = true;
                }
            } else {
                tracing::warn!("TraceLlm hint mismatch: no user message found");
                mismatch = true;
            }
        }

        if let Some(min_count) = hint.min_message_count {
            if messages.len() < min_count {
                tracing::warn!(
                    "TraceLlm hint mismatch: expected >= {} messages, got {}",
                    min_count,
                    messages.len()
                );
                mismatch = true;
            }
        }

        if mismatch {
            self.hint_mismatches.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[async_trait]
impl LlmProvider for TraceLlm {
    fn model_name(&self) -> &str {
        &self.trace.model_name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        self.captured_requests
            .lock()
            .unwrap()
            .push(request.messages.clone());

        let step = self.next_step()?;

        if let Some(ref hint) = step.request_hint {
            self.validate_hint(hint, &request.messages);
        }

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
                provider: self.trace.model_name.clone(),
                reason: "TraceLlm: complete() called but step has tool_calls; use complete_with_tools()".to_string(),
            }),
        }
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        self.captured_requests
            .lock()
            .unwrap()
            .push(request.messages.clone());

        let step = self.next_step()?;

        if let Some(ref hint) = step.request_hint {
            self.validate_hint(hint, &request.messages);
        }

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
            } => Ok(ToolCompletionResponse {
                content: None,
                tool_calls: tool_calls
                    .into_iter()
                    .map(|tc| ToolCall {
                        id: tc.id,
                        name: tc.name,
                        arguments: tc.arguments,
                    })
                    .collect(),
                input_tokens,
                output_tokens,
                finish_reason: FinishReason::ToolUse,
            }),
        }
    }
}
```

**Step 4: Create test fixture file**

Create `tests/fixtures/llm_traces/simple_text.json`:
```json
{
  "model_name": "test-model",
  "steps": [
    {
      "response": {
        "type": "text",
        "content": "Hello from fixture file!",
        "input_tokens": 50,
        "output_tokens": 10
      }
    }
  ]
}
```

**Step 5: Run tests to verify they pass**

Run: `cargo test -p ironclaw --test support -- 2>&1`

Note: Integration tests in `tests/` must import from `ironclaw::` (the crate), not `crate::`. The tests file structure for Rust integration tests means each `tests/*.rs` file is a separate crate. The `tests/support/` dir is shared via `mod support;` in each test file.

Actually, the `tests/support/` files won't compile as standalone tests. They need to be imported by a test file. We'll use them in Task 3's test file. For now, ensure the module compiles by creating a thin integration test.

Create `tests/trace_llm_tests.rs`:
```rust
mod support;

use support::trace_llm::*;
// Tests are inside support/trace_llm.rs
```

Run: `cargo test --test trace_llm_tests`
Expected: All 7 tests pass.

**Step 6: Commit**

```bash
git add tests/support/ tests/fixtures/ tests/trace_llm_tests.rs
git commit -m "feat: add TraceLlm provider for replay-based E2E testing"
```

---

## Task 2: TestChannel

Build a `Channel` implementation for injecting messages and capturing responses/status events in-process.

**Files:**
- Create: `tests/support/test_channel.rs`
- Modify: `tests/support/mod.rs` -- add `pub mod test_channel;`

**Step 1: Write the failing test**

Add to `tests/support/test_channel.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw::channels::IncomingMessage;

    #[tokio::test]
    async fn test_channel_send_and_receive_message() {
        let channel = TestChannel::new();
        let mut stream = channel.start().await.unwrap();

        // Inject a message
        channel.send_message("Hello agent").await;

        // Should appear in the stream
        use futures::StreamExt;
        let msg = stream.next().await.unwrap();
        assert_eq!(msg.content, "Hello agent");
        assert_eq!(msg.channel, "test");
        assert_eq!(msg.user_id, "test-user");
    }

    #[tokio::test]
    async fn test_channel_captures_responses() {
        let channel = TestChannel::new();
        let msg = IncomingMessage::new("test", "test-user", "Hi");

        channel
            .respond(&msg, OutgoingResponse::text("Hello back"))
            .await
            .unwrap();

        let responses = channel.captured_responses();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].content, "Hello back");
    }

    #[tokio::test]
    async fn test_channel_captures_status_events() {
        let channel = TestChannel::new();

        channel
            .send_status(
                StatusUpdate::ToolStarted {
                    name: "time".to_string(),
                },
                &serde_json::Value::Null,
            )
            .await
            .unwrap();

        channel
            .send_status(
                StatusUpdate::ToolCompleted {
                    name: "time".to_string(),
                    success: true,
                },
                &serde_json::Value::Null,
            )
            .await
            .unwrap();

        let events = channel.captured_status_events();
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn test_channel_auto_approves() {
        let channel = TestChannel::new();

        channel
            .send_status(
                StatusUpdate::ApprovalNeeded {
                    request_id: "req-1".to_string(),
                    tool_name: "shell".to_string(),
                    description: "Run ls".to_string(),
                    parameters: serde_json::json!({}),
                },
                &serde_json::Value::Null,
            )
            .await
            .unwrap();

        // Auto-approve should inject an approval message into the stream
        // (TestChannel handles this internally)
        let events = channel.captured_status_events();
        assert!(events.iter().any(|e| matches!(e, StatusUpdate::ApprovalNeeded { .. })));
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --test trace_llm_tests` (after adding test_channel to mod.rs)
Expected: Compilation error -- `TestChannel` not defined.

**Step 3: Write the implementation**

```rust
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;
use tokio::sync::{mpsc, Mutex};

use ironclaw::channels::{
    Channel, ChannelError, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
};

/// A test channel for in-process E2E testing.
///
/// - Inject messages via `send_message()`
/// - Capture responses via `captured_responses()`
/// - Capture status events via `captured_status_events()`
/// - Auto-approves all tool execution requests
pub struct TestChannel {
    tx: mpsc::Sender<IncomingMessage>,
    rx: Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
    responses: Arc<Mutex<Vec<OutgoingResponse>>>,
    status_events: Arc<Mutex<Vec<StatusUpdate>>>,
    user_id: String,
}

impl TestChannel {
    pub fn new() -> Self {
        Self::with_user_id("test-user")
    }

    pub fn with_user_id(user_id: impl Into<String>) -> Self {
        let (tx, rx) = mpsc::channel(64);
        Self {
            tx,
            rx: Mutex::new(Some(rx)),
            responses: Arc::new(Mutex::new(Vec::new())),
            status_events: Arc::new(Mutex::new(Vec::new())),
            user_id: user_id.into(),
        }
    }

    /// Inject a user message into the agent loop.
    pub async fn send_message(&self, content: &str) {
        let msg = IncomingMessage::new("test", &self.user_id, content);
        self.tx.send(msg).await.expect("TestChannel send failed");
    }

    /// Inject a user message with a specific thread ID.
    pub async fn send_message_in_thread(&self, content: &str, thread_id: &str) {
        let msg = IncomingMessage::new("test", &self.user_id, content)
            .with_thread(thread_id);
        self.tx.send(msg).await.expect("TestChannel send failed");
    }

    /// Get all captured responses (cloned).
    pub fn captured_responses(&self) -> Vec<OutgoingResponse> {
        // Use try_lock for sync contexts, block for async
        self.responses.try_lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Wait for at least `n` responses with a timeout.
    pub async fn wait_for_responses(&self, n: usize, timeout: std::time::Duration) -> Vec<OutgoingResponse> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            {
                let responses = self.responses.lock().await;
                if responses.len() >= n {
                    return responses.clone();
                }
            }
            if tokio::time::Instant::now() >= deadline {
                let responses = self.responses.lock().await;
                return responses.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// Get all captured status events (cloned).
    pub fn captured_status_events(&self) -> Vec<StatusUpdate> {
        self.status_events.try_lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Get captured tool-started events.
    pub async fn tool_calls_started(&self) -> Vec<String> {
        self.status_events
            .lock()
            .await
            .iter()
            .filter_map(|e| match e {
                StatusUpdate::ToolStarted { name } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    /// Get captured tool-completed events.
    pub async fn tool_calls_completed(&self) -> Vec<(String, bool)> {
        self.status_events
            .lock()
            .await
            .iter()
            .filter_map(|e| match e {
                StatusUpdate::ToolCompleted { name, success } => {
                    Some((name.clone(), *success))
                }
                _ => None,
            })
            .collect()
    }

    /// Clear all captured data.
    pub async fn clear(&self) {
        self.responses.lock().await.clear();
        self.status_events.lock().await.clear();
    }
}

#[async_trait]
impl Channel for TestChannel {
    fn name(&self) -> &str {
        "test"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let rx = self
            .rx
            .lock()
            .await
            .take()
            .ok_or_else(|| ChannelError::StartupFailed {
                name: "test".to_string(),
                reason: "TestChannel.start() already called".to_string(),
            })?;
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.responses.lock().await.push(response);
        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        self.status_events.lock().await.push(status);
        Ok(())
    }

    async fn broadcast(
        &self,
        _user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.responses.lock().await.push(response);
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}
```

**Step 4: Run tests**

Run: `cargo test --test trace_llm_tests`
Expected: All tests pass.

**Step 5: Commit**

```bash
git add tests/support/test_channel.rs tests/support/mod.rs
git commit -m "feat: add TestChannel for in-process E2E message injection and capture"
```

---

## Task 3: TestRig Builder

Wire `TraceLlm` + `TestChannel` + `Agent` + libSQL into a reusable test rig.

**Files:**
- Create: `tests/support/test_rig.rs`
- Modify: `tests/support/mod.rs` -- add `pub mod test_rig;`

**Step 1: Write the failing test**

Add to `tests/support/test_rig.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::trace_llm::{LlmTrace, TraceStep, TraceResponse};

    #[tokio::test]
    async fn test_rig_builds_and_runs() {
        let trace = LlmTrace {
            model_name: "test-model".to_string(),
            steps: vec![TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: "Hello from the agent!".to_string(),
                    input_tokens: 50,
                    output_tokens: 10,
                },
            }],
        };

        let rig = TestRig::builder()
            .with_trace(trace)
            .build()
            .await;

        // Send a message and wait for response
        rig.send_message("Hi").await;
        let responses = rig.wait_for_responses(1, std::time::Duration::from_secs(10)).await;

        assert!(!responses.is_empty(), "Should receive at least one response");
        // The response should contain the text from the trace
        assert!(
            responses.iter().any(|r| r.content.contains("Hello from the agent")),
            "Response should contain trace text, got: {:?}",
            responses.iter().map(|r| &r.content).collect::<Vec<_>>()
        );

        rig.shutdown().await;
    }
}
```

**Step 2: Run tests to verify they fail**

Expected: Compilation error -- `TestRig` not defined.

**Step 3: Write the implementation**

```rust
use std::sync::Arc;

use ironclaw::agent::agent_loop::Agent;
use ironclaw::agent::AgentDeps;
use ironclaw::channels::manager::ChannelManager;
use ironclaw::channels::OutgoingResponse;
use ironclaw::config::AgentConfig;
use ironclaw::llm::LlmProvider;
use ironclaw::testing::TestHarnessBuilder;
use ironclaw::tools::ToolRegistry;

use super::test_channel::TestChannel;
use super::trace_llm::{LlmTrace, TraceLlm};

/// Assembled E2E test rig with a running Agent.
pub struct TestRig {
    channel: Arc<TestChannel>,
    agent_handle: tokio::task::JoinHandle<()>,
    #[allow(dead_code)]
    harness_guard: TestRigGuard,
}

/// Holds resources that must outlive the agent (temp dir, etc).
struct TestRigGuard {
    _temp_dir: tempfile::TempDir,
}

impl TestRig {
    pub fn builder() -> TestRigBuilder {
        TestRigBuilder::new()
    }

    /// Inject a user message.
    pub async fn send_message(&self, content: &str) {
        self.channel.send_message(content).await;
    }

    /// Wait for at least `n` responses.
    pub async fn wait_for_responses(
        &self,
        n: usize,
        timeout: std::time::Duration,
    ) -> Vec<OutgoingResponse> {
        self.channel.wait_for_responses(n, timeout).await
    }

    /// Get all captured tool-started event names.
    pub async fn tool_calls_started(&self) -> Vec<String> {
        self.channel.tool_calls_started().await
    }

    /// Get all captured tool-completed events.
    pub async fn tool_calls_completed(&self) -> Vec<(String, bool)> {
        self.channel.tool_calls_completed().await
    }

    /// Get all captured status events.
    pub fn captured_status_events(&self) -> Vec<ironclaw::channels::StatusUpdate> {
        self.channel.captured_status_events()
    }

    /// Clear captured data for next turn.
    pub async fn clear(&self) {
        self.channel.clear().await;
    }

    /// Shut down the agent loop.
    pub async fn shutdown(self) {
        self.agent_handle.abort();
        let _ = self.agent_handle.await;
    }
}

pub struct TestRigBuilder {
    trace: Option<LlmTrace>,
    llm: Option<Arc<dyn LlmProvider>>,
    tools: Option<Arc<ToolRegistry>>,
    max_tool_iterations: usize,
}

impl TestRigBuilder {
    pub fn new() -> Self {
        Self {
            trace: None,
            llm: None,
            tools: None,
            max_tool_iterations: 10,
        }
    }

    /// Set the LLM trace to replay.
    pub fn with_trace(mut self, trace: LlmTrace) -> Self {
        self.trace = Some(trace);
        self
    }

    /// Override the LLM provider directly (instead of using a trace).
    pub fn with_llm(mut self, llm: Arc<dyn LlmProvider>) -> Self {
        self.llm = Some(llm);
        self
    }

    /// Override the tool registry.
    pub fn with_tools(mut self, tools: Arc<ToolRegistry>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Set max tool iterations per agentic loop.
    pub fn with_max_tool_iterations(mut self, n: usize) -> Self {
        self.max_tool_iterations = n;
        self
    }

    /// Build the rig: creates Agent, starts it in a background task.
    #[cfg(feature = "libsql")]
    pub async fn build(self) -> TestRig {
        use ironclaw::agent::cost_guard::{CostGuard, CostGuardConfig};
        use ironclaw::config::{SafetyConfig, SkillsConfig};
        use ironclaw::hooks::HookRegistry;
        use ironclaw::safety::SafetyLayer;
        use ironclaw::db::libsql::LibSqlBackend;

        // Database
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let db_path = temp_dir.path().join("test.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create test db");
        backend.run_migrations().await.expect("run migrations");
        let db: Arc<dyn ironclaw::db::Database> = Arc::new(backend);

        // LLM
        let llm: Arc<dyn LlmProvider> = if let Some(llm) = self.llm {
            llm
        } else if let Some(trace) = self.trace {
            Arc::new(TraceLlm::from_trace(trace))
        } else {
            Arc::new(ironclaw::testing::StubLlm::default())
        };

        // Tools
        let tools = self.tools.unwrap_or_else(|| {
            let t = Arc::new(ToolRegistry::new());
            t.register_builtin_tools();
            t
        });

        // Safety (permissive for tests)
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
        }));

        let hooks = Arc::new(HookRegistry::new());
        let cost_guard = Arc::new(CostGuard::new(CostGuardConfig {
            max_cost_per_day_cents: None,
            max_actions_per_hour: None,
        }));

        let deps = AgentDeps {
            store: Some(Arc::clone(&db)),
            llm,
            cheap_llm: None,
            safety,
            tools,
            workspace: None,
            extension_manager: None,
            skill_registry: None,
            skill_catalog: None,
            skills_config: SkillsConfig::default(),
            hooks,
            cost_guard,
        };

        // Channel
        let test_channel = Arc::new(TestChannel::new());
        let channel_manager = Arc::new(ChannelManager::new());
        // TestChannel needs to be added as a Box<dyn Channel>. We need to
        // create a wrapper or use Arc-based approach.
        // ChannelManager.add() takes Box<dyn Channel>, but we need to keep
        // a reference. Solution: TestChannel uses Arc internally for shared state.
        channel_manager
            .add(Box::new(TestChannelHandle::new(Arc::clone(&test_channel))))
            .await;

        // Agent config (test-friendly)
        let config = AgentConfig {
            name: "test-agent".to_string(),
            max_parallel_jobs: 1,
            job_timeout: std::time::Duration::from_secs(60),
            stuck_threshold: std::time::Duration::from_secs(300),
            repair_check_interval: std::time::Duration::from_secs(3600), // Effectively disabled
            max_repair_attempts: 0,
            use_planning: false,
            session_idle_timeout: std::time::Duration::from_secs(3600),
            allow_local_tools: true,
            max_cost_per_day_cents: None,
            max_actions_per_hour: None,
            max_tool_iterations: self.max_tool_iterations,
            auto_approve_tools: true,
        };

        let agent = Agent::new(
            config,
            deps,
            channel_manager,
            None, // heartbeat
            None, // hygiene
            None, // routines
            None, // context_manager
            None, // session_manager
        );

        // Run agent in background
        let agent_handle = tokio::spawn(async move {
            if let Err(e) = agent.run().await {
                tracing::error!("Test agent exited with error: {}", e);
            }
        });

        // Give the agent a moment to start listening
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        TestRig {
            channel: test_channel,
            agent_handle,
            harness_guard: TestRigGuard {
                _temp_dir: temp_dir,
            },
        }
    }
}

/// A thin handle that delegates to a shared TestChannel.
/// Needed because ChannelManager::add() takes Box<dyn Channel>
/// but we need to keep a reference for sending messages and reading captures.
struct TestChannelHandle {
    inner: Arc<TestChannel>,
}

impl TestChannelHandle {
    fn new(inner: Arc<TestChannel>) -> Self {
        Self { inner }
    }
}

#[async_trait::async_trait]
impl ironclaw::channels::Channel for TestChannelHandle {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn start(&self) -> Result<ironclaw::channels::MessageStream, ironclaw::channels::ChannelError> {
        self.inner.start().await
    }

    async fn respond(
        &self,
        msg: &ironclaw::channels::IncomingMessage,
        response: ironclaw::channels::OutgoingResponse,
    ) -> Result<(), ironclaw::channels::ChannelError> {
        self.inner.respond(msg, response).await
    }

    async fn send_status(
        &self,
        status: ironclaw::channels::StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ironclaw::channels::ChannelError> {
        self.inner.send_status(status, metadata).await
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: ironclaw::channels::OutgoingResponse,
    ) -> Result<(), ironclaw::channels::ChannelError> {
        self.inner.broadcast(user_id, response).await
    }

    async fn health_check(&self) -> Result<(), ironclaw::channels::ChannelError> {
        self.inner.health_check().await
    }
}
```

Note: The exact import paths and type visibility may need adjustment during implementation. The key pattern is:
- `TestChannel` holds the shared state (Arc<Mutex<...>>)
- `TestChannelHandle` wraps `Arc<TestChannel>` and implements `Channel`
- `ChannelManager` gets the handle, test code keeps the `Arc<TestChannel>`

**Step 4: Run tests**

Run: `cargo test --test trace_llm_tests test_rig`
Expected: `test_rig_builds_and_runs` passes.

**Step 5: Commit**

```bash
git add tests/support/test_rig.rs tests/support/mod.rs
git commit -m "feat: add TestRig builder for wiring E2E test agent"
```

---

## Task 4: E2E Trace Test -- Memory Flow

First real E2E test: user message triggers `memory_write`, then a follow-up retrieves it.

**Files:**
- Create: `tests/fixtures/llm_traces/memory_write_read.json`
- Create: `tests/e2e_trace_memory.rs`

**Step 1: Create the trace fixture**

`tests/fixtures/llm_traces/memory_write_read.json`:
```json
{
  "model_name": "trace-model",
  "steps": [
    {
      "request_hint": {
        "last_user_message_contains": "save a note"
      },
      "response": {
        "type": "tool_calls",
        "tool_calls": [
          {
            "id": "call_write_1",
            "name": "memory_write",
            "arguments": {
              "path": "notes/project-alpha.md",
              "content": "# Project Alpha\n\nLaunches on March 15th."
            }
          }
        ],
        "input_tokens": 200,
        "output_tokens": 50
      }
    },
    {
      "response": {
        "type": "text",
        "content": "I've saved a note about Project Alpha launching on March 15th.",
        "input_tokens": 300,
        "output_tokens": 30
      }
    }
  ]
}
```

**Step 2: Write the test**

`tests/e2e_trace_memory.rs`:
```rust
mod support;

use support::test_rig::TestRig;
use support::trace_llm::LlmTrace;

#[cfg(feature = "libsql")]
#[tokio::test]
async fn test_memory_write_flow() {
    let trace =
        LlmTrace::from_file("tests/fixtures/llm_traces/memory_write_read.json")
            .expect("load trace");

    let rig = TestRig::builder()
        .with_trace(trace)
        .build()
        .await;

    // Send user message
    rig.send_message("Please save a note: Project Alpha launches on March 15th")
        .await;

    // Wait for agent response
    let responses = rig
        .wait_for_responses(1, std::time::Duration::from_secs(15))
        .await;

    // Assertions
    assert!(!responses.is_empty(), "Should get a response");
    assert!(
        responses.iter().any(|r| r.content.contains("Project Alpha")),
        "Response should mention Project Alpha"
    );

    // Verify tool was called
    let tools_started = rig.tool_calls_started().await;
    assert!(
        tools_started.contains(&"memory_write".to_string()),
        "memory_write tool should have been called, got: {:?}",
        tools_started
    );

    // Verify tool completed successfully
    let tools_completed = rig.tool_calls_completed().await;
    assert!(
        tools_completed.iter().any(|(name, success)| name == "memory_write" && *success),
        "memory_write should complete successfully"
    );

    rig.shutdown().await;
}
```

**Step 3: Run test**

Run: `cargo test --test e2e_trace_memory`
Expected: Pass. The trace drives the LLM to call `memory_write`, the real tool executes against the libSQL database, the agent returns the canned text response.

**Step 4: Commit**

```bash
git add tests/e2e_trace_memory.rs tests/fixtures/llm_traces/memory_write_read.json
git commit -m "test: add E2E trace test for memory_write flow"
```

---

## Task 5: E2E Trace Test -- File Tools

Test that the agent can call `write_file` then `read_file` on a workspace path.

**Files:**
- Create: `tests/fixtures/llm_traces/file_write_read.json`
- Create: `tests/e2e_trace_file_tools.rs`

**Step 1: Create trace fixture**

`tests/fixtures/llm_traces/file_write_read.json`:
```json
{
  "model_name": "trace-model",
  "steps": [
    {
      "request_hint": {
        "last_user_message_contains": "write"
      },
      "response": {
        "type": "tool_calls",
        "tool_calls": [
          {
            "id": "call_write",
            "name": "write_file",
            "arguments": {
              "path": "/tmp/ironclaw_test_e2e/hello.txt",
              "content": "Hello, E2E test!"
            }
          }
        ],
        "input_tokens": 150,
        "output_tokens": 40
      }
    },
    {
      "response": {
        "type": "tool_calls",
        "tool_calls": [
          {
            "id": "call_read",
            "name": "read_file",
            "arguments": {
              "path": "/tmp/ironclaw_test_e2e/hello.txt"
            }
          }
        ],
        "input_tokens": 250,
        "output_tokens": 30
      }
    },
    {
      "response": {
        "type": "text",
        "content": "I wrote 'Hello, E2E test!' to the file and read it back successfully.",
        "input_tokens": 350,
        "output_tokens": 25
      }
    }
  ]
}
```

**Step 2: Write the test**

`tests/e2e_trace_file_tools.rs`:
```rust
mod support;

use support::test_rig::TestRig;
use support::trace_llm::LlmTrace;

#[cfg(feature = "libsql")]
#[tokio::test]
async fn test_file_write_and_read_flow() {
    // Clean up test dir
    let test_dir = std::path::Path::new("/tmp/ironclaw_test_e2e");
    let _ = std::fs::remove_dir_all(test_dir);
    std::fs::create_dir_all(test_dir).expect("create test dir");

    let trace =
        LlmTrace::from_file("tests/fixtures/llm_traces/file_write_read.json")
            .expect("load trace");

    let rig = TestRig::builder()
        .with_trace(trace)
        .build()
        .await;

    rig.send_message("Write 'Hello, E2E test!' to /tmp/ironclaw_test_e2e/hello.txt then read it back")
        .await;

    let responses = rig
        .wait_for_responses(1, std::time::Duration::from_secs(15))
        .await;

    assert!(!responses.is_empty(), "Should get a response");

    // Verify the file was actually written
    let content = std::fs::read_to_string("/tmp/ironclaw_test_e2e/hello.txt")
        .expect("file should exist");
    assert_eq!(content, "Hello, E2E test!");

    // Verify both tools were called
    let tools = rig.tool_calls_started().await;
    assert!(tools.contains(&"write_file".to_string()), "write_file called");
    assert!(tools.contains(&"read_file".to_string()), "read_file called");

    // Clean up
    let _ = std::fs::remove_dir_all(test_dir);
    rig.shutdown().await;
}
```

**Step 3: Run test**

Run: `cargo test --test e2e_trace_file_tools`
Expected: Pass.

**Step 4: Commit**

```bash
git add tests/e2e_trace_file_tools.rs tests/fixtures/llm_traces/file_write_read.json
git commit -m "test: add E2E trace test for file write/read flow"
```

---

## Task 6: E2E Trace Test -- Error Path

Test that invalid tool parameters produce a graceful error, not a crash.

**Files:**
- Create: `tests/fixtures/llm_traces/error_path.json`
- Create: `tests/e2e_trace_error_path.rs`

**Step 1: Create trace fixture**

`tests/fixtures/llm_traces/error_path.json`:
```json
{
  "model_name": "trace-model",
  "steps": [
    {
      "response": {
        "type": "tool_calls",
        "tool_calls": [
          {
            "id": "call_bad",
            "name": "read_file",
            "arguments": {}
          }
        ],
        "input_tokens": 100,
        "output_tokens": 20
      }
    },
    {
      "response": {
        "type": "text",
        "content": "I encountered an error trying to read the file. The path parameter was missing.",
        "input_tokens": 200,
        "output_tokens": 30
      }
    }
  ]
}
```

**Step 2: Write the test**

`tests/e2e_trace_error_path.rs`:
```rust
mod support;

use support::test_rig::TestRig;
use support::trace_llm::LlmTrace;

#[cfg(feature = "libsql")]
#[tokio::test]
async fn test_tool_error_handled_gracefully() {
    let trace =
        LlmTrace::from_file("tests/fixtures/llm_traces/error_path.json")
            .expect("load trace");

    let rig = TestRig::builder()
        .with_trace(trace)
        .build()
        .await;

    rig.send_message("Read a file for me").await;

    let responses = rig
        .wait_for_responses(1, std::time::Duration::from_secs(15))
        .await;

    // Agent should respond (not crash)
    assert!(!responses.is_empty(), "Agent should respond even on tool error");

    // The tool should have been attempted
    let tools = rig.tool_calls_started().await;
    assert!(tools.contains(&"read_file".to_string()));

    // The tool should have failed
    let completed = rig.tool_calls_completed().await;
    let read_result = completed.iter().find(|(name, _)| name == "read_file");
    assert!(
        read_result.is_some(),
        "read_file should appear in completed events"
    );
    // Success might be false due to missing params
    // (depends on how the tool reports the error)

    rig.shutdown().await;
}
```

**Step 3: Run test**

Run: `cargo test --test e2e_trace_error_path`
Expected: Pass.

**Step 4: Commit**

```bash
git add tests/e2e_trace_error_path.rs tests/fixtures/llm_traces/error_path.json
git commit -m "test: add E2E trace test for tool error handling"
```

---

## Task 7: CI Wiring

Add the E2E trace tests to CI.

**Files:**
- Modify: `.github/workflows/test.yml` (or equivalent CI config)

**Step 1: Identify existing CI config**

Read the existing test workflow to understand the structure.

**Step 2: Add E2E trace test step**

Add a step that runs:
```yaml
- name: E2E Trace Tests
  run: cargo test --test e2e_trace_* --features libsql -- --nocapture
  env:
    RUST_LOG: ironclaw=debug
    RUST_TEST_THREADS: 1
```

Key settings:
- `--features libsql` -- needed for the libSQL test database
- `RUST_TEST_THREADS=1` -- reduce contention between tests sharing temp dirs
- `--nocapture` -- show trace output for debugging

**Step 3: Run CI locally to verify**

Run: `cargo test --test e2e_trace_memory --test e2e_trace_file_tools --test e2e_trace_error_path --features libsql`
Expected: All pass.

**Step 4: Commit**

```bash
git add .github/workflows/test.yml
git commit -m "ci: add E2E trace tests to CI workflow"
```

---

## Implementation Notes

### Import Path Adjustments

Integration tests in `tests/` import from the crate as `ironclaw::`, not `crate::`. All references to internal types need the full crate path. If certain types aren't `pub`, they'll need to be made public or the test support code needs to live in `src/` (e.g., `src/testing/`).

**Types that MUST be pub for integration tests:**
- `ironclaw::agent::agent_loop::Agent` -- verify `Agent::new()` and `Agent::run()` are pub
- `ironclaw::agent::AgentDeps` -- verify pub
- `ironclaw::channels::manager::ChannelManager` -- verify pub
- `ironclaw::config::AgentConfig` -- verify pub
- `ironclaw::config::SafetyConfig` -- verify pub

If any of these aren't pub, the simplest fix is to make them pub. The alternative is to move `TestRig` into `src/testing.rs` behind `#[cfg(test)]` or a `testing` feature flag.

### What If Visibility Is Blocked?

If `Agent::new()` or `AgentConfig` fields are `pub(crate)`, we have two options:

1. **Move test support into `src/testing/`** -- keeps test code inside the crate boundary. Add `mod test_rig;` etc. to `src/testing.rs`.

2. **Add a `testing` feature flag** -- `#[cfg(feature = "testing")] pub ...` to expose internals only when tests are built.

Option 1 is simpler and matches the existing pattern (`src/testing.rs` already exists).

### Record Mode (Future Enhancement)

Illia's plan mentions `RECORD_LLM_TRACE=1` for recording traces from real LLM calls. This is a future enhancement:
- When `RECORD_LLM_TRACE=1`, `TraceLlm` delegates to a real provider and captures responses
- Written to a trace JSON file for later replay
- Not needed for the initial implementation

### Docker/Sandbox Tests (Future Enhancement)

Task for shell tool with Docker sandbox (`e2e_trace_shell_sandbox.rs`) is deferred. It requires:
- Docker available in CI
- `REQUIRE_DOCKER=1` env var
- SandboxManager initialization

This can be added once the basic rig is working.
