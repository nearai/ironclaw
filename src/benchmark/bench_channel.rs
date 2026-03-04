//! Minimal channel implementation for benchmark scenarios.
//!
//! Captures agent responses and tool status events without TUI or HTTP overhead.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::{Mutex, Notify, mpsc};

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;

/// Minimal channel for benchmark execution.
pub struct BenchChannel {
    rx: Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
    responses: Arc<Mutex<Vec<OutgoingResponse>>>,
    status_events: Arc<Mutex<Vec<StatusUpdate>>>,
    tool_start_times: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    tool_timings: Arc<Mutex<Vec<(String, u64)>>>,
    response_notify: Arc<Notify>,
}

impl BenchChannel {
    pub fn new(rx: mpsc::Receiver<IncomingMessage>) -> Self {
        Self {
            rx: Mutex::new(Some(rx)),
            responses: Arc::new(Mutex::new(Vec::new())),
            status_events: Arc::new(Mutex::new(Vec::new())),
            tool_start_times: Arc::new(Mutex::new(HashMap::new())),
            tool_timings: Arc::new(Mutex::new(Vec::new())),
            response_notify: Arc::new(Notify::new()),
        }
    }

    /// Wait for the agent to produce a text response.
    pub async fn wait_for_response(&self) -> String {
        loop {
            let responses = self.responses.lock().await;
            if let Some(r) = responses.last() {
                return r.content.clone();
            }
            drop(responses);
            self.response_notify.notified().await;
        }
    }

    /// Return (name, success) for all completed tool calls.
    pub async fn tool_calls_completed(&self) -> Vec<(String, bool)> {
        self.status_events
            .lock()
            .await
            .iter()
            .filter_map(|s| match s {
                StatusUpdate::ToolCompleted { name, success } => Some((name.clone(), *success)),
                _ => None,
            })
            .collect()
    }

    /// Return (name, duration_ms) for all timed tool calls.
    pub async fn tool_timings(&self) -> Vec<(String, u64)> {
        self.tool_timings.lock().await.clone()
    }
}

#[async_trait]
impl Channel for BenchChannel {
    fn name(&self) -> &str {
        "benchmark"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let rx = self
            .rx
            .lock()
            .await
            .take()
            .ok_or(ChannelError::StartupFailed {
                name: "benchmark".to_string(),
                reason: "start() already called".to_string(),
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
        self.response_notify.notify_waiters();
        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
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

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

/// Handle wrapper for ChannelManager (same pattern as TestChannelHandle).
pub struct BenchChannelHandle {
    inner: Arc<BenchChannel>,
}

impl BenchChannelHandle {
    pub fn new(inner: Arc<BenchChannel>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl Channel for BenchChannelHandle {
    fn name(&self) -> &str {
        self.inner.name()
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
    async fn health_check(&self) -> Result<(), ChannelError> {
        self.inner.health_check().await
    }
}
