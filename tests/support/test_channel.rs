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
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;

use ironclaw::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use ironclaw::error::ChannelError;

/// Captured outbound event in the order it was emitted.
#[derive(Clone, Debug)]
pub enum CapturedEvent {
    Response(OutgoingResponse),
    Status(StatusUpdate),
}

// ---------------------------------------------------------------------------
// TestChannel
// ---------------------------------------------------------------------------

/// A `Channel` implementation for injecting messages and capturing responses
/// in integration tests.
pub struct TestChannel {
    /// Channel name returned by `Channel::name()`.
    channel_name: String,
    /// Sender half for injecting `IncomingMessage`s into the stream.
    tx: mpsc::Sender<IncomingMessage>,
    /// Receiver half, wrapped in Option so `start()` can take it exactly once.
    rx: Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
    /// Captured outgoing responses.
    pub responses: Arc<Mutex<Vec<OutgoingResponse>>>,
    /// Captured status events.
    status_events: Arc<Mutex<Vec<StatusUpdate>>>,
    /// Ordered log of responses and status events.
    captured_events: Arc<Mutex<Vec<CapturedEvent>>>,
    /// Tracks when each tool started (by name). Supports nested/overlapping tools
    /// by using a Vec of start times per tool name.
    tool_start_times: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    /// Completed tool timings: (name, duration_ms).
    tool_timings: Arc<Mutex<Vec<(String, u64)>>>,
    /// Default user ID for injected messages.
    user_id: String,
    /// Shutdown signal: when set to `true`, signals the agent to stop.
    shutdown: Arc<AtomicBool>,
    /// Sender half of the ready signal, fired when `start()` is called.
    ready_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    /// Receiver half of the ready signal, taken by the test rig before awaiting.
    ready_rx: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
}

impl TestChannel {
    /// Create a new TestChannel with the default user ID "test-user".
    pub fn new() -> Self {
        Self::with_user_id("test-user")
    }

    /// Create a new TestChannel with a custom user ID.
    pub fn with_user_id(user_id: impl Into<String>) -> Self {
        let (tx, rx) = mpsc::channel(256);
        let (ready_tx, ready_rx) = oneshot::channel();
        Self {
            channel_name: "test".to_string(),
            tx,
            rx: Mutex::new(Some(rx)),
            responses: Arc::new(Mutex::new(Vec::new())),
            status_events: Arc::new(Mutex::new(Vec::new())),
            captured_events: Arc::new(Mutex::new(Vec::new())),
            tool_start_times: Arc::new(Mutex::new(HashMap::new())),
            tool_timings: Arc::new(Mutex::new(Vec::new())),
            user_id: user_id.into(),
            shutdown: Arc::new(AtomicBool::new(false)),
            ready_tx: Arc::new(Mutex::new(Some(ready_tx))),
            ready_rx: Arc::new(Mutex::new(Some(ready_rx))),
        }
    }

    /// Override the channel name (default: "test").
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.channel_name = name.into();
        self
    }

    /// Signal the channel (and any listening agent) to shut down.
    pub fn signal_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Take the ready signal receiver. Returns `None` if already taken.
    ///
    /// The receiver resolves when the agent calls `start()` on this channel,
    /// providing a race-free alternative to sleep-based startup waits.
    pub async fn take_ready_rx(&self) -> Option<oneshot::Receiver<()>> {
        self.ready_rx.lock().await.take()
    }

    /// Inject a user message into the channel stream.
    pub async fn send_message(&self, content: &str) {
        let msg = IncomingMessage::new(&self.channel_name, &self.user_id, content);
        self.tx.send(msg).await.expect("TestChannel tx closed");
    }

    /// Inject a raw `IncomingMessage` (for tests that need attachments, etc.).
    pub async fn send_incoming(&self, msg: IncomingMessage) {
        self.tx.send(msg).await.expect("TestChannel tx closed");
    }

    /// Inject a user message with a specific thread ID.
    pub async fn send_message_in_thread(&self, content: &str, thread_id: &str) {
        let msg =
            IncomingMessage::new(&self.channel_name, &self.user_id, content).with_thread(thread_id);
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

    /// Async version of `captured_responses` — safe to call while the agent is
    /// actively pushing responses (avoids `try_lock` panic on contention).
    pub async fn captured_responses_async(&self) -> Vec<OutgoingResponse> {
        self.responses.lock().await.clone()
    }

    /// Wait until at least `n` responses have been captured, or `timeout` elapses.
    ///
    /// Returns whatever responses have been collected when the condition is met
    /// or the timeout expires. Uses exponential backoff (50ms -> 100ms -> 200ms,
    /// capped at 500ms) to reduce lock contention while staying responsive.
    pub async fn wait_for_responses(&self, n: usize, timeout: Duration) -> Vec<OutgoingResponse> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut interval = Duration::from_millis(50);
        let max_interval = Duration::from_millis(500);
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
            tokio::time::sleep(interval).await;
            interval = (interval * 2).min(max_interval);
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

    /// Return the ordered log of emitted outbound events.
    pub fn captured_events(&self) -> Vec<CapturedEvent> {
        self.captured_events
            .try_lock()
            .expect("captured_events lock contention")
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
                StatusUpdate::ToolCompleted { name, success, .. } => Some((name.clone(), *success)),
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
        self.captured_events.lock().await.clear();
        self.tool_start_times.lock().await.clear();
        self.tool_timings.lock().await.clear();
    }
}

// ---------------------------------------------------------------------------
// TestChannelHandle -- wraps Arc<TestChannel> as Box<dyn Channel>
// ---------------------------------------------------------------------------

/// A thin wrapper around `Arc<TestChannel>` that implements `Channel`.
///
/// This lets us hand a `Box<dyn Channel>` to `ChannelManager::add()` while
/// keeping an `Arc<TestChannel>` in the test rig for sending messages and
/// reading captures. The `name_override` allows different test harnesses
/// to present the channel under different names (e.g. "gateway" vs "test").
pub struct TestChannelHandle {
    inner: Arc<TestChannel>,
    name: String,
}

impl TestChannelHandle {
    /// Create a handle that delegates `name()` to the inner `TestChannel`.
    pub fn new(inner: Arc<TestChannel>) -> Self {
        Self {
            name: inner.name().to_string(),
            inner,
        }
    }

    /// Create a handle with a custom channel name.
    pub fn with_name(inner: Arc<TestChannel>, name: impl Into<String>) -> Self {
        Self {
            inner,
            name: name.into(),
        }
    }
}

#[async_trait]
impl Channel for TestChannelHandle {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        self.inner.start().await
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.inner.respond(msg, response).await
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        self.inner.send_status(status, metadata).await
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.inner.broadcast(user_id, response).await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        self.inner.health_check().await
    }

    fn conversation_context(&self, metadata: &serde_json::Value) -> HashMap<String, String> {
        self.inner.conversation_context(metadata)
    }
}

// ---------------------------------------------------------------------------
// Channel trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl Channel for TestChannel {
    fn name(&self) -> &str {
        &self.channel_name
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let rx = self
            .rx
            .lock()
            .await
            .take()
            .ok_or_else(|| ChannelError::StartupFailed {
                name: self.channel_name.clone(),
                reason: "start() already called".to_string(),
            })?;

        let stream = ReceiverStream::new(rx).boxed();

        // Signal that the channel has started and the agent is ready.
        if let Some(tx) = self.ready_tx.lock().await.take() {
            let _ = tx.send(());
        }

        Ok(stream)
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.responses.lock().await.push(response.clone());
        self.captured_events
            .lock()
            .await
            .push(CapturedEvent::Response(response));
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
        self.status_events.lock().await.push(status.clone());
        self.captured_events
            .lock()
            .await
            .push(CapturedEvent::Status(status));
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
