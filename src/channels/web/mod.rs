//! Web gateway channel for browser-based access to IronClaw.
//!
//! Provides a single-page web UI with:
//! - Chat with the agent (via REST + SSE)
//! - Workspace/memory browsing
//! - Job management
//!
//! ```text
//! Browser ─── POST /api/chat/send ──► Agent Loop
//!         ◄── GET  /api/chat/events ── SSE stream
//!         ─── GET  /api/memory/* ────► Workspace
//!         ─── GET  /api/jobs/* ──────► ContextManager
//!         ◄── GET  / ───────────────── Static HTML/CSS/JS
//! ```

pub mod auth;
pub mod log_layer;
pub mod server;
pub mod sse;
pub mod types;

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::agent::SessionManager;
use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::config::GatewayConfig;
use crate::context::ContextManager;
use crate::error::ChannelError;
use crate::extensions::ExtensionManager;
use crate::tools::ToolRegistry;
use crate::workspace::Workspace;

use self::log_layer::LogBroadcaster;

use self::server::GatewayState;
use self::sse::SseManager;
use self::types::SseEvent;

/// Web gateway channel implementing the Channel trait.
pub struct GatewayChannel {
    config: GatewayConfig,
    state: Arc<GatewayState>,
    /// The actual auth token in use (generated or from config).
    auth_token: String,
}

impl GatewayChannel {
    /// Create a new gateway channel.
    ///
    /// If no auth token is configured, generates a random one and prints it.
    pub fn new(config: GatewayConfig) -> Self {
        let auth_token = config.auth_token.clone().unwrap_or_else(|| {
            use rand::Rng;
            let token: String = rand::thread_rng()
                .sample_iter(&rand::distributions::Alphanumeric)
                .take(32)
                .map(char::from)
                .collect();
            token
        });

        let state = Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: SseManager::new(),
            workspace: None,
            context_manager: None,
            session_manager: None,
            log_broadcaster: None,
            extension_manager: None,
            tool_registry: None,
            user_id: config.user_id.clone(),
            shutdown_tx: tokio::sync::RwLock::new(None),
        });

        Self {
            config,
            state,
            auth_token,
        }
    }

    /// Helper to rebuild state, copying existing fields and applying a mutation.
    fn rebuild_state(&mut self, mutate: impl FnOnce(&mut GatewayState)) {
        let mut new_state = GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: SseManager::new(),
            workspace: self.state.workspace.clone(),
            context_manager: self.state.context_manager.clone(),
            session_manager: self.state.session_manager.clone(),
            log_broadcaster: self.state.log_broadcaster.clone(),
            extension_manager: self.state.extension_manager.clone(),
            tool_registry: self.state.tool_registry.clone(),
            user_id: self.state.user_id.clone(),
            shutdown_tx: tokio::sync::RwLock::new(None),
        };
        mutate(&mut new_state);
        self.state = Arc::new(new_state);
    }

    /// Inject the workspace reference for the memory API.
    pub fn with_workspace(mut self, workspace: Arc<Workspace>) -> Self {
        self.rebuild_state(|s| s.workspace = Some(workspace));
        self
    }

    /// Inject the context manager for the jobs API.
    pub fn with_context_manager(mut self, cm: Arc<ContextManager>) -> Self {
        self.rebuild_state(|s| s.context_manager = Some(cm));
        self
    }

    /// Inject the session manager for thread/session info.
    pub fn with_session_manager(mut self, sm: Arc<SessionManager>) -> Self {
        self.rebuild_state(|s| s.session_manager = Some(sm));
        self
    }

    /// Inject the log broadcaster for the logs SSE endpoint.
    pub fn with_log_broadcaster(mut self, lb: Arc<LogBroadcaster>) -> Self {
        self.rebuild_state(|s| s.log_broadcaster = Some(lb));
        self
    }

    /// Inject the extension manager for the extensions API.
    pub fn with_extension_manager(mut self, em: Arc<ExtensionManager>) -> Self {
        self.rebuild_state(|s| s.extension_manager = Some(em));
        self
    }

    /// Inject the tool registry for the extensions API.
    pub fn with_tool_registry(mut self, tr: Arc<ToolRegistry>) -> Self {
        self.rebuild_state(|s| s.tool_registry = Some(tr));
        self
    }

    /// Get the auth token (for printing to console on startup).
    pub fn auth_token(&self) -> &str {
        &self.auth_token
    }

    /// Get a reference to the shared gateway state (for the agent to push SSE events).
    pub fn state(&self) -> &Arc<GatewayState> {
        &self.state
    }
}

#[async_trait]
impl Channel for GatewayChannel {
    fn name(&self) -> &str {
        "gateway"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(256);
        *self.state.msg_tx.write().await = Some(tx);

        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port)
            .parse()
            .map_err(|e| ChannelError::StartupFailed {
                name: "gateway".to_string(),
                reason: format!(
                    "Invalid address '{}:{}': {}",
                    self.config.host, self.config.port, e
                ),
            })?;

        server::start_server(addr, self.state.clone(), self.auth_token.clone()).await?;

        tracing::info!("Web gateway listening on http://{}", addr);
        tracing::info!("Auth token: {}", self.auth_token);

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let thread_id = msg.thread_id.clone().unwrap_or_default();

        self.state.sse.broadcast(SseEvent::Response {
            content: response.content,
            thread_id,
        });

        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let event = match status {
            StatusUpdate::Thinking(msg) => SseEvent::Thinking { message: msg },
            StatusUpdate::ToolStarted { name } => SseEvent::ToolStarted { name },
            StatusUpdate::ToolCompleted { name, success } => {
                SseEvent::ToolCompleted { name, success }
            }
            StatusUpdate::ToolResult { name, preview } => SseEvent::ToolResult { name, preview },
            StatusUpdate::StreamChunk(content) => SseEvent::StreamChunk { content },
            StatusUpdate::Status(msg) => SseEvent::Status { message: msg },
            StatusUpdate::ApprovalNeeded {
                request_id,
                tool_name,
                description,
                parameters,
            } => SseEvent::ApprovalNeeded {
                request_id,
                tool_name,
                description,
                parameters: serde_json::to_string_pretty(&parameters)
                    .unwrap_or_else(|_| parameters.to_string()),
            },
        };

        self.state.sse.broadcast(event);
        Ok(())
    }

    async fn broadcast(
        &self,
        _user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.state.sse.broadcast(SseEvent::Response {
            content: response.content,
            thread_id: String::new(),
        });
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        if self.state.msg_tx.read().await.is_some() {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: "gateway".to_string(),
            })
        }
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        if let Some(tx) = self.state.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        }
        *self.state.msg_tx.write().await = None;
        Ok(())
    }
}
