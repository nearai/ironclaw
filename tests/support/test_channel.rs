//! TestChannel -- an in-process Channel for E2E testing.
//!
//! Injects messages into the agent loop via an mpsc sender and captures
//! responses and status events for assertion in tests.

#![allow(dead_code)] // Public API consumed by later test modules (Task 3+).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use ironclaw::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use ironclaw::error::ChannelError;

// ---------------------------------------------------------------------------
// TestChannel
// ---------------------------------------------------------------------------

/// A `Channel` implementation for injecting messages and capturing responses
/// in integration tests.
pub struct TestChannel {
    /// Sender half for injecting `IncomingMessage`s into the stream.
    tx: mpsc::Sender<IncomingMessage>,
    /// Receiver half, wrapped in Option so `start()` can take it exactly once.
    rx: Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
    /// Captured outgoing responses.
    responses: Arc<Mutex<Vec<OutgoingResponse>>>,
    /// Captured status events.
    status_events: Arc<Mutex<Vec<StatusUpdate>>>,
    /// Tracks when each tool started (by name). Supports nested/overlapping tools
    /// by using a Vec of start times per tool name.
    tool_start_times: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    /// Completed tool timings: (name, duration_ms).
    tool_timings: Arc<Mutex<Vec<(String, u64)>>>,
    /// Default user ID for injected messages.
    user_id: String,
    /// Shutdown signal: when set to `true`, signals the agent to stop.
    shutdown: Arc<AtomicBool>,
}

impl TestChannel {
    /// Create a new TestChannel with the default user ID "test-user".
    pub fn new() -> Self {
        Self::with_user_id("test-user")
    }

    /// Create a new TestChannel with a custom user ID.
    pub fn with_user_id(user_id: impl Into<String>) -> Self {
        let (tx, rx) = mpsc::channel(256);
        Self {
            tx,
            rx: Mutex::new(Some(rx)),
            responses: Arc::new(Mutex::new(Vec::new())),
            status_events: Arc::new(Mutex::new(Vec::new())),
            tool_start_times: Arc::new(Mutex::new(HashMap::new())),
            tool_timings: Arc::new(Mutex::new(Vec::new())),
            user_id: user_id.into(),
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Signal the channel (and any listening agent) to shut down.
    pub fn signal_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Inject a user message into the channel stream.
    pub async fn send_message(&self, content: &str) {
        let msg = IncomingMessage::new("test", &self.user_id, content);
        self.tx.send(msg).await.expect("TestChannel tx closed");
    }

    /// Inject a user message with a specific thread ID.
    pub async fn send_message_in_thread(&self, content: &str, thread_id: &str) {
        let msg = IncomingMessage::new("test", &self.user_id, content).with_thread(thread_id);
        self.tx.send(msg).await.expect("TestChannel tx closed");
    }

    /// Return a snapshot of all captured responses.
    ///
    /// Uses `try_lock` so it can be called from sync contexts in tests.
    pub fn captured_responses(&self) -> Vec<OutgoingResponse> {
        self.responses
            .try_lock()
            .expect("captured_responses lock contention")
            .clone()
    }

    /// Wait until at least `n` responses have been captured, or `timeout` elapses.
    ///
    /// Returns whatever responses have been collected when the condition is met
    /// or the timeout expires.
    pub async fn wait_for_responses(&self, n: usize, timeout: Duration) -> Vec<OutgoingResponse> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            {
                let guard = self.responses.lock().await;
                if guard.len() >= n {
                    return guard.clone();
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return self.responses.lock().await.clone();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Return a snapshot of all captured status events.
    ///
    /// Uses `try_lock` so it can be called from sync contexts in tests.
    pub fn captured_status_events(&self) -> Vec<StatusUpdate> {
        self.status_events
            .try_lock()
            .expect("captured_status_events lock contention")
            .clone()
    }

    /// Return the names of all `ToolStarted` events captured so far.
    pub fn tool_calls_started(&self) -> Vec<String> {
        self.captured_status_events()
            .iter()
            .filter_map(|s| match s {
                StatusUpdate::ToolStarted { name } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    /// Return `(name, success)` for all `ToolCompleted` events captured so far.
    pub fn tool_calls_completed(&self) -> Vec<(String, bool)> {
        self.captured_status_events()
            .iter()
            .filter_map(|s| match s {
                StatusUpdate::ToolCompleted { name, success } => Some((name.clone(), *success)),
                _ => None,
            })
            .collect()
    }

    /// Return `(name, preview)` for all `ToolResult` events captured so far.
    pub fn tool_results(&self) -> Vec<(String, String)> {
        self.captured_status_events()
            .iter()
            .filter_map(|s| match s {
                StatusUpdate::ToolResult { name, preview } => Some((name.clone(), preview.clone())),
                _ => None,
            })
            .collect()
    }

    /// Return `(name, duration_ms)` for all completed tools with timing data.
    ///
    /// Uses `try_lock` so it can be called from sync contexts in tests.
    pub fn tool_timings(&self) -> Vec<(String, u64)> {
        self.tool_timings
            .try_lock()
            .expect("tool_timings lock contention")
            .clone()
    }

    /// Clear all captured responses and status events.
    pub async fn clear(&self) {
        self.responses.lock().await.clear();
        self.status_events.lock().await.clear();
        self.tool_start_times.lock().await.clear();
        self.tool_timings.lock().await.clear();
    }
}

// ---------------------------------------------------------------------------
// Channel trait implementation
// ---------------------------------------------------------------------------

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
                reason: "start() already called".to_string(),
            })?;

        let stream = ReceiverStream::new(rx).boxed();
        Ok(stream)
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
        // Capture timing before pushing to events.
        match &status {
            StatusUpdate::ToolStarted { name } => {
                self.tool_start_times
                    .lock()
                    .await
                    .entry(name.clone())
                    .or_default()
                    .push(Instant::now());
            }
            StatusUpdate::ToolCompleted { name, .. } => {
                if let Some(starts) = self.tool_start_times.lock().await.get_mut(name)
                    && let Some(start) = starts.pop()
                {
                    self.tool_timings
                        .lock()
                        .await
                        .push((name.clone(), start.elapsed().as_millis() as u64));
                }
            }
            _ => {}
        }
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

    fn conversation_context(&self, _metadata: &serde_json::Value) -> HashMap<String, String> {
        HashMap::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // 1. Send a message and read it back from the stream.
    #[tokio::test]
    async fn test_channel_send_and_receive_message() {
        let channel = TestChannel::new();
        let mut stream = channel.start().await.unwrap();

        channel.send_message("hello world").await;

        let msg = stream.next().await.expect("stream should yield a message");
        assert_eq!(msg.content, "hello world");
        assert_eq!(msg.channel, "test");
        assert_eq!(msg.user_id, "test-user");
    }

    // 2. Responses are captured via respond().
    #[tokio::test]
    async fn test_channel_captures_responses() {
        let channel = TestChannel::new();
        let incoming = IncomingMessage::new("test", "test-user", "hi");

        channel
            .respond(&incoming, OutgoingResponse::text("reply 1"))
            .await
            .unwrap();
        channel
            .respond(&incoming, OutgoingResponse::text("reply 2"))
            .await
            .unwrap();

        let captured = channel.captured_responses();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].content, "reply 1");
        assert_eq!(captured[1].content, "reply 2");
    }

    // 3. Status events are captured via send_status().
    #[tokio::test]
    async fn test_channel_captures_status_events() {
        let channel = TestChannel::new();
        let metadata = serde_json::Value::Null;

        channel
            .send_status(
                StatusUpdate::ToolStarted {
                    name: "echo".to_string(),
                },
                &metadata,
            )
            .await
            .unwrap();
        channel
            .send_status(
                StatusUpdate::ToolCompleted {
                    name: "echo".to_string(),
                    success: true,
                },
                &metadata,
            )
            .await
            .unwrap();

        let events = channel.captured_status_events();
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], StatusUpdate::ToolStarted { name } if name == "echo"));
        assert!(
            matches!(&events[1], StatusUpdate::ToolCompleted { name, success } if name == "echo" && *success)
        );
    }

    // 4. tool_calls_started() filters ToolStarted events.
    #[tokio::test]
    async fn test_channel_tool_calls_started() {
        let channel = TestChannel::new();
        let metadata = serde_json::Value::Null;

        channel
            .send_status(
                StatusUpdate::ToolStarted {
                    name: "memory_search".to_string(),
                },
                &metadata,
            )
            .await
            .unwrap();
        channel
            .send_status(StatusUpdate::Thinking("hmm".to_string()), &metadata)
            .await
            .unwrap();
        channel
            .send_status(
                StatusUpdate::ToolStarted {
                    name: "echo".to_string(),
                },
                &metadata,
            )
            .await
            .unwrap();

        let started = channel.tool_calls_started();
        assert_eq!(started, vec!["memory_search", "echo"]);
    }

    // 5. tool_results() filters ToolResult events.
    #[tokio::test]
    async fn test_channel_tool_results() {
        let channel = TestChannel::new();
        channel
            .send_status(
                StatusUpdate::ToolResult {
                    name: "echo".to_string(),
                    preview: "hello world".to_string(),
                },
                &serde_json::Value::Null,
            )
            .await
            .unwrap();
        channel
            .send_status(
                StatusUpdate::ToolResult {
                    name: "time".to_string(),
                    preview: "{\"iso\": \"2026-03-03\"}".to_string(),
                },
                &serde_json::Value::Null,
            )
            .await
            .unwrap();

        let results = channel.tool_results();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "echo");
        assert_eq!(results[0].1, "hello world");
        assert_eq!(results[1].0, "time");
        assert!(results[1].1.contains("2026"));
    }

    // 6. wait_for_responses() collects responses that arrive after a delay.
    #[tokio::test]
    async fn test_channel_wait_for_responses() {
        let channel = TestChannel::new();
        let responses = Arc::clone(&channel.responses);

        // Spawn a task that pushes a response after 100ms.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            responses
                .lock()
                .await
                .push(OutgoingResponse::text("delayed reply"));
        });

        let collected = channel.wait_for_responses(1, Duration::from_secs(2)).await;
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].content, "delayed reply");
    }

    // 7. tool_timings() captures real elapsed time between ToolStarted and ToolCompleted.
    #[tokio::test]
    async fn test_channel_tool_timings() {
        let channel = TestChannel::new();
        channel
            .send_status(
                StatusUpdate::ToolStarted {
                    name: "echo".to_string(),
                },
                &serde_json::Value::Null,
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        channel
            .send_status(
                StatusUpdate::ToolCompleted {
                    name: "echo".to_string(),
                    success: true,
                },
                &serde_json::Value::Null,
            )
            .await
            .unwrap();

        let timings = channel.tool_timings();
        assert_eq!(timings.len(), 1);
        assert_eq!(timings[0].0, "echo");
        // Should be >= 40ms (50ms sleep minus some scheduling variance).
        assert!(
            timings[0].1 >= 40,
            "Expected >= 40ms, got {}ms",
            timings[0].1
        );
    }
}
