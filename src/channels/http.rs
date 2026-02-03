//! HTTP webhook channel for receiving messages via HTTP POST.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse};
use crate::config::HttpConfig;
use crate::error::ChannelError;

/// HTTP webhook channel.
pub struct HttpChannel {
    config: HttpConfig,
    state: Arc<HttpChannelState>,
}

struct HttpChannelState {
    /// Sender for incoming messages.
    tx: RwLock<Option<mpsc::Sender<IncomingMessage>>>,
    /// Pending responses keyed by message ID.
    pending_responses: RwLock<std::collections::HashMap<Uuid, oneshot::Sender<String>>>,
    /// Server shutdown signal.
    shutdown_tx: RwLock<Option<oneshot::Sender<()>>>,
}

impl HttpChannel {
    /// Create a new HTTP channel.
    pub fn new(config: HttpConfig) -> Self {
        Self {
            config,
            state: Arc::new(HttpChannelState {
                tx: RwLock::new(None),
                pending_responses: RwLock::new(std::collections::HashMap::new()),
                shutdown_tx: RwLock::new(None),
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
struct WebhookRequest {
    /// User or client identifier.
    user_id: String,
    /// Message content.
    content: String,
    /// Optional thread ID for conversation tracking.
    thread_id: Option<String>,
    /// Optional webhook secret for authentication.
    secret: Option<String>,
    /// Whether to wait for a synchronous response.
    #[serde(default)]
    wait_for_response: bool,
}

#[derive(Debug, Serialize)]
struct WebhookResponse {
    /// Message ID assigned to this request.
    message_id: Uuid,
    /// Status of the request.
    status: String,
    /// Response content (only if wait_for_response was true).
    response: Option<String>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: String,
    channel: String,
}

async fn health_handler() -> impl IntoResponse {
    Json(HealthResponse {
        status: "healthy".to_string(),
        channel: "http".to_string(),
    })
}

async fn webhook_handler(
    State(state): State<Arc<HttpChannelState>>,
    Json(req): Json<WebhookRequest>,
) -> impl IntoResponse {
    // TODO: Validate secret if configured

    let msg =
        IncomingMessage::new("http", &req.user_id, &req.content).with_metadata(serde_json::json!({
            "wait_for_response": req.wait_for_response,
        }));

    if let Some(thread_id) = &req.thread_id {
        let msg = msg.with_thread(thread_id);
        return process_message(state, msg, req.wait_for_response).await;
    }

    process_message(state, msg, req.wait_for_response).await
}

async fn process_message(
    state: Arc<HttpChannelState>,
    msg: IncomingMessage,
    wait_for_response: bool,
) -> impl IntoResponse {
    let msg_id = msg.id;

    // Set up response channel if waiting
    let response_rx = if wait_for_response {
        let (tx, rx) = oneshot::channel();
        state.pending_responses.write().await.insert(msg_id, tx);
        Some(rx)
    } else {
        None
    };

    // Send message to the channel
    let tx_guard = state.tx.read().await;
    if let Some(tx) = tx_guard.as_ref() {
        if tx.send(msg).await.is_err() {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(WebhookResponse {
                    message_id: msg_id,
                    status: "error".to_string(),
                    response: Some("Channel closed".to_string()),
                }),
            );
        }
    } else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(WebhookResponse {
                message_id: msg_id,
                status: "error".to_string(),
                response: Some("Channel not started".to_string()),
            }),
        );
    }
    drop(tx_guard);

    // Wait for response if requested
    let response = if let Some(rx) = response_rx {
        match tokio::time::timeout(std::time::Duration::from_secs(60), rx).await {
            Ok(Ok(content)) => Some(content),
            Ok(Err(_)) => Some("Response cancelled".to_string()),
            Err(_) => Some("Response timeout".to_string()),
        }
    } else {
        None
    };

    (
        StatusCode::OK,
        Json(WebhookResponse {
            message_id: msg_id,
            status: "accepted".to_string(),
            response,
        }),
    )
}

#[async_trait]
impl Channel for HttpChannel {
    fn name(&self) -> &str {
        "http"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(256);
        *self.state.tx.write().await = Some(tx);

        let state = self.state.clone();
        let host = self.config.host.clone();
        let port = self.config.port;

        // Create router
        let app = Router::new()
            .route("/health", get(health_handler))
            .route("/webhook", post(webhook_handler))
            .with_state(state.clone());

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        *self.state.shutdown_tx.write().await = Some(shutdown_tx);

        // Spawn server
        tokio::spawn(async move {
            let addr: SocketAddr = format!("{}:{}", host, port)
                .parse()
                .expect("Invalid address");

            tracing::info!("HTTP channel listening on {}", addr);

            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();

            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                    tracing::info!("HTTP channel shutting down");
                })
                .await
                .unwrap();
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Check if there's a pending response waiter
        if let Some(tx) = self.state.pending_responses.write().await.remove(&msg.id) {
            let _ = tx.send(response.content);
        }
        // For async webhooks, we'd need to make an HTTP callback here
        // but that requires the caller to provide a callback URL
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        // Check if we have an active sender
        if self.state.tx.read().await.is_some() {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: "http".to_string(),
            })
        }
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        // Send shutdown signal
        if let Some(tx) = self.state.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        }
        // Clear the message sender
        *self.state.tx.write().await = None;
        Ok(())
    }
}
