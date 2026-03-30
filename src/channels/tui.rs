//! TUI channel — bridges the `Channel` trait to `ironclaw_tui`.
//!
//! The TUI crate owns the terminal and event loop. This module translates
//! between the agent's `Channel` trait and `ironclaw_tui`'s event/message
//! channels.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use ironclaw_tui::{TuiAppConfig, TuiEvent, TuiLayout, start_tui};

use crate::channels::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
};
use crate::channels::web::log_layer::LogBroadcaster;
use crate::error::ChannelError;

/// TUI channel backed by `ironclaw_tui`.
pub struct TuiChannel {
    user_id: String,
    event_tx: Arc<Mutex<Option<mpsc::Sender<TuiEvent>>>>,
    started: AtomicBool,
    version: String,
    model: String,
    layout: TuiLayout,
    log_broadcaster: Option<Arc<LogBroadcaster>>,
}

impl TuiChannel {
    /// Create a new TUI channel.
    pub fn new(
        user_id: impl Into<String>,
        version: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            user_id: user_id.into(),
            event_tx: Arc::new(Mutex::new(None)),
            started: AtomicBool::new(false),
            version: version.into(),
            model: model.into(),
            layout: TuiLayout::default(),
            log_broadcaster: None,
        }
    }

    /// Set the layout configuration.
    pub fn with_layout(mut self, layout: TuiLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Set the log broadcaster for forwarding log entries to the TUI.
    pub fn with_log_broadcaster(mut self, broadcaster: Arc<LogBroadcaster>) -> Self {
        self.log_broadcaster = Some(broadcaster);
        self
    }
}

#[async_trait]
impl Channel for TuiChannel {
    fn name(&self) -> &str {
        "tui"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        if self.started.swap(true, Ordering::Relaxed) {
            return Err(ChannelError::StartupFailed {
                name: "tui".to_string(),
                reason: "TUI channel already started".to_string(),
            });
        }

        let config = TuiAppConfig {
            version: self.version.clone(),
            model: self.model.clone(),
            layout: self.layout.clone(),
            context_window: 128_000,
        };

        let ironclaw_tui::TuiAppHandle {
            event_tx,
            mut msg_rx,
            join_handle: _join,
        } = start_tui(config);

        // Store event_tx for sending status updates and responses
        *self.event_tx.lock().await = Some(event_tx.clone());

        // Forward log entries from the LogBroadcaster to the TUI's Logs tab
        if let Some(ref broadcaster) = self.log_broadcaster {
            // Replay recent history first
            let log_tx = event_tx.clone();
            for entry in broadcaster.recent_entries() {
                let _ = log_tx
                    .send(TuiEvent::Log {
                        level: entry.level,
                        target: entry.target,
                        message: entry.message,
                        timestamp: entry.timestamp,
                    })
                    .await;
            }

            // Subscribe to live log stream
            let mut log_rx = broadcaster.subscribe();
            tokio::spawn(async move {
                while let Ok(entry) = log_rx.recv().await {
                    let event = TuiEvent::Log {
                        level: entry.level,
                        target: entry.target,
                        message: entry.message,
                        timestamp: entry.timestamp,
                    };
                    if log_tx.send(event).await.is_err() {
                        break;
                    }
                }
            });
        }

        // Bridge: forward user messages from TUI to the agent's MessageStream
        let (incoming_tx, incoming_rx) = mpsc::channel::<IncomingMessage>(32);
        let user_id = self.user_id.clone();
        let sys_tz = crate::timezone::detect_system_timezone()
            .name()
            .to_string();

        tokio::spawn(async move {
            while let Some(text) = msg_rx.recv().await {
                let msg =
                    IncomingMessage::new("tui", &user_id, &text).with_timezone(&sys_tz);
                if incoming_tx.send(msg).await.is_err() {
                    break;
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(incoming_rx)))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        if let Some(ref tx) = *self.event_tx.lock().await {
            let _ = tx
                .send(TuiEvent::Response {
                    content: response.content,
                    thread_id: response.thread_id,
                })
                .await;
        }
        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let tx_guard = self.event_tx.lock().await;
        let Some(ref tx) = *tx_guard else {
            return Ok(());
        };

        let event = match status {
            StatusUpdate::Thinking(msg) => TuiEvent::Thinking(msg),
            StatusUpdate::ToolStarted { name } => TuiEvent::ToolStarted { name },
            StatusUpdate::ToolCompleted {
                name,
                success,
                error,
                ..
            } => TuiEvent::ToolCompleted {
                name,
                success,
                error,
            },
            StatusUpdate::ToolResult { name, preview } => {
                TuiEvent::ToolResult { name, preview }
            }
            StatusUpdate::StreamChunk(chunk) => TuiEvent::StreamChunk(chunk),
            StatusUpdate::Status(msg) => TuiEvent::Status(msg),
            StatusUpdate::JobStarted {
                job_id, title, ..
            } => TuiEvent::JobStarted { job_id, title },
            StatusUpdate::ApprovalNeeded {
                request_id,
                tool_name,
                description,
                parameters,
                allow_always,
            } => TuiEvent::ApprovalNeeded {
                request_id,
                tool_name,
                description,
                parameters,
                allow_always,
            },
            StatusUpdate::AuthRequired {
                extension_name,
                instructions,
                ..
            } => TuiEvent::AuthRequired {
                extension_name,
                instructions,
            },
            StatusUpdate::AuthCompleted {
                extension_name,
                success,
                message,
            } => TuiEvent::AuthCompleted {
                extension_name,
                success,
                message,
            },
            StatusUpdate::ReasoningUpdate {
                narrative,
                decisions: _,
            } => TuiEvent::ReasoningUpdate { narrative },
            StatusUpdate::TurnCost {
                input_tokens,
                output_tokens,
                cost_usd,
            } => TuiEvent::TurnCost {
                input_tokens,
                output_tokens,
                cost_usd,
            },
            StatusUpdate::Suggestions { suggestions } => {
                TuiEvent::Suggestions { suggestions }
            }
            StatusUpdate::ImageGenerated { .. } => return Ok(()),
        };

        let _ = tx.send(event).await;
        Ok(())
    }

    async fn broadcast(
        &self,
        _user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        if let Some(ref tx) = *self.event_tx.lock().await {
            let _ = tx
                .send(TuiEvent::Response {
                    content: response.content,
                    thread_id: response.thread_id,
                })
                .await;
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        // The TUI thread will exit when event channels are dropped
        Ok(())
    }
}
