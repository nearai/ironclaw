use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use ironclaw::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use ironclaw::error::ChannelError;

use crate::results::TraceToolCall;
use crate::suite::ConversationTurn;

/// Captured state from a benchmark channel run.
#[derive(Debug, Default)]
pub struct ChannelCapture {
    /// All responses the agent sent back.
    pub responses: Vec<String>,
    /// Tool calls observed (name, success, duration_ms).
    pub tool_calls: Vec<TraceToolCall>,
    /// Full conversation turns for multi-turn scoring.
    pub conversation: Vec<ConversationTurn>,
    /// Status messages (for debugging).
    pub status_log: Vec<String>,
}

/// A headless Channel implementation for benchmarking.
///
/// Modeled after `ReplChannel`: uses mpsc to inject messages and captures
/// all responses and tool status events. Auto-approves tool execution
/// so benchmarks run without user interaction.
pub struct BenchChannel {
    /// Sender to inject messages into the agent loop.
    msg_tx: mpsc::Sender<IncomingMessage>,
    /// Receiver the agent loop reads from (taken once by `start()`).
    msg_rx: Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
    /// Accumulated capture data.
    capture: Arc<Mutex<ChannelCapture>>,
}

impl BenchChannel {
    pub fn new() -> (Self, mpsc::Sender<IncomingMessage>) {
        let (tx, rx) = mpsc::channel(64);
        let channel = Self {
            msg_tx: tx.clone(),
            msg_rx: Mutex::new(Some(rx)),
            capture: Arc::new(Mutex::new(ChannelCapture::default())),
        };
        (channel, tx)
    }

    /// Get a handle to the capture data.
    pub fn capture(&self) -> Arc<Mutex<ChannelCapture>> {
        Arc::clone(&self.capture)
    }

    /// Get a clone of the message sender for injecting follow-up messages.
    pub fn sender(&self) -> mpsc::Sender<IncomingMessage> {
        self.msg_tx.clone()
    }
}

#[async_trait]
impl Channel for BenchChannel {
    fn name(&self) -> &str {
        "bench"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let rx = self
            .msg_rx
            .lock()
            .await
            .take()
            .ok_or_else(|| ChannelError::StartupFailed {
                name: "bench".to_string(),
                reason: "start() already called".to_string(),
            })?;
        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let mut cap = self.capture.lock().await;
        cap.responses.push(response.content.clone());
        cap.conversation.push(ConversationTurn {
            role: crate::suite::TurnRole::Assistant,
            content: response.content,
        });
        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let mut cap = self.capture.lock().await;

        match status {
            StatusUpdate::ToolCompleted { ref name, success } => {
                cap.tool_calls.push(TraceToolCall {
                    name: name.clone(),
                    duration_ms: 0, // We don't have precise per-tool timing here
                    success,
                });
                cap.status_log
                    .push(format!("tool_completed: {name} success={success}"));
            }
            StatusUpdate::ApprovalNeeded { ref request_id, .. } => {
                // Auto-approve all tools during benchmarks
                cap.status_log.push(format!("auto_approved: {request_id}"));
                drop(cap); // Release lock before sending
                let approval = IncomingMessage::new("bench", "bench-user", "always");
                let _ = self.msg_tx.send(approval).await;
                return Ok(());
            }
            StatusUpdate::Thinking(ref msg) => {
                cap.status_log.push(format!("thinking: {msg}"));
            }
            StatusUpdate::ToolStarted { ref name } => {
                cap.status_log.push(format!("tool_started: {name}"));
            }
            StatusUpdate::ToolResult {
                ref name,
                ref preview,
            } => {
                cap.status_log.push(format!(
                    "tool_result: {name} -> {}",
                    &preview[..preview.len().min(100)]
                ));
            }
            StatusUpdate::StreamChunk(_) => {}
            StatusUpdate::Status(ref msg) => {
                cap.status_log.push(format!("status: {msg}"));
            }
            StatusUpdate::JobStarted {
                ref job_id,
                ref title,
                ..
            } => {
                cap.status_log
                    .push(format!("job_started: {job_id} ({title})"));
            }
            StatusUpdate::AuthRequired {
                ref extension_name, ..
            } => {
                cap.status_log
                    .push(format!("auth_required: {extension_name} (auto-skipped)"));
            }
            StatusUpdate::AuthCompleted {
                ref extension_name,
                success,
                ..
            } => {
                cap.status_log
                    .push(format!("auth_completed: {extension_name} success={success}"));
            }
        }
        Ok(())
    }

    async fn broadcast(
        &self,
        _user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let mut cap = self.capture.lock().await;
        cap.status_log.push(format!(
            "broadcast: {}",
            &response.content[..response.content.len().min(100)]
        ));
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bench_channel_captures_responses() {
        let (channel, _tx) = BenchChannel::new();
        let capture = channel.capture();

        let msg = IncomingMessage::new("bench", "user", "hello");
        let response = OutgoingResponse::text("world");
        channel.respond(&msg, response).await.unwrap();

        let cap = capture.lock().await;
        assert_eq!(cap.responses.len(), 1);
        assert_eq!(cap.responses[0], "world");
        assert_eq!(cap.conversation.len(), 1);
    }

    #[tokio::test]
    async fn test_bench_channel_auto_approves() {
        let (channel, _tx) = BenchChannel::new();
        // start() to consume the receiver
        let _stream = channel.start().await.unwrap();

        let status = StatusUpdate::ApprovalNeeded {
            request_id: "req-1".to_string(),
            tool_name: "shell".to_string(),
            description: "run ls".to_string(),
            parameters: serde_json::json!({}),
        };
        channel
            .send_status(status, &serde_json::Value::Null)
            .await
            .unwrap();

        // The approval message was sent through msg_tx,
        // which means the stream would receive it.
        // We can't easily read from the stream in this test without
        // consuming it, but we can verify the status log.
        let capture_arc = channel.capture();
        let cap = capture_arc.lock().await;
        assert!(cap.status_log.iter().any(|s| s.contains("auto_approved")));
    }

    #[tokio::test]
    async fn test_bench_channel_captures_tool_events() {
        let (channel, _tx) = BenchChannel::new();

        let status = StatusUpdate::ToolCompleted {
            name: "echo".to_string(),
            success: true,
        };
        channel
            .send_status(status, &serde_json::Value::Null)
            .await
            .unwrap();

        let capture_arc = channel.capture();
        let cap = capture_arc.lock().await;
        assert_eq!(cap.tool_calls.len(), 1);
        assert_eq!(cap.tool_calls[0].name, "echo");
        assert!(cap.tool_calls[0].success);
    }
}
