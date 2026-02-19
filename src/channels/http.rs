//! HTTP webhook channel for receiving messages via HTTP POST.

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
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
    /// Expected webhook secret for authentication (if configured).
    webhook_secret: Option<String>,
    /// Fixed user ID for this HTTP channel.
    user_id: String,
    /// Rate limiting state.
    rate_limit: tokio::sync::Mutex<RateLimitState>,
}

#[derive(Debug)]
struct RateLimitState {
    window_start: std::time::Instant,
    request_count: u32,
}

/// Maximum JSON body size for webhook requests (64 KB).
const MAX_BODY_BYTES: usize = 64 * 1024;

/// Maximum number of pending wait-for-response requests.
const MAX_PENDING_RESPONSES: usize = 100;

/// Maximum requests per minute.
const MAX_REQUESTS_PER_MINUTE: u32 = 60;

/// Maximum content length for a single message.
const MAX_CONTENT_BYTES: usize = 32 * 1024;

impl HttpChannel {
    /// Create a new HTTP channel.
    pub fn new(config: HttpConfig) -> Self {
        let webhook_secret = config
            .webhook_secret
            .as_ref()
            .map(|s| s.expose_secret().to_string());
        let user_id = config.user_id.clone();

        Self {
            config,
            state: Arc::new(HttpChannelState {
                tx: RwLock::new(None),
                pending_responses: RwLock::new(std::collections::HashMap::new()),
                webhook_secret,
                user_id,
                rate_limit: tokio::sync::Mutex::new(RateLimitState {
                    window_start: std::time::Instant::now(),
                    request_count: 0,
                }),
            }),
        }
    }

    /// Return the channel's axum routes with state applied.
    ///
    /// The returned `Router` shares the same `Arc<HttpChannelState>` that
    /// `start()` later populates. Before `start()` is called the webhook
    /// handler returns 503 ("Channel not started").
    pub fn routes(&self) -> Router {
        Router::new()
            .route("/health", get(health_handler))
            .route("/webhook", post(webhook_handler))
            .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
            .with_state(self.state.clone())
    }

    /// Return the configured host and port for this channel.
    pub fn addr(&self) -> (&str, u16) {
        (&self.config.host, self.config.port)
    }
}

#[derive(Debug, Deserialize)]
struct WebhookRequest {
    /// User or client identifier (ignored, user is fixed by server config).
    #[serde(default)]
    user_id: Option<String>,
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
) -> (StatusCode, Json<WebhookResponse>) {
    // Rate limiting
    {
        let mut limiter = state.rate_limit.lock().await;
        if limiter.window_start.elapsed() >= std::time::Duration::from_secs(60) {
            limiter.window_start = std::time::Instant::now();
            limiter.request_count = 0;
        }
        limiter.request_count += 1;
        if limiter.request_count > MAX_REQUESTS_PER_MINUTE {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(WebhookResponse {
                    message_id: Uuid::nil(),
                    status: "error".to_string(),
                    response: Some("Rate limit exceeded".to_string()),
                }),
            );
        }
    }

    let _ = req.user_id.as_ref().map(|user_id| {
        tracing::debug!(
            provided_user_id = %user_id,
            "HTTP webhook request provided user_id, ignoring in favor of configured user_id"
        );
    });

    // Validate secret if configured
    if let Some(ref expected_secret) = state.webhook_secret {
        match &req.secret {
            Some(provided) if bool::from(provided.as_bytes().ct_eq(expected_secret.as_bytes())) => {
                // Secret matches, continue
            }
            Some(_) => {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(WebhookResponse {
                        message_id: Uuid::nil(),
                        status: "error".to_string(),
                        response: Some("Invalid webhook secret".to_string()),
                    }),
                );
            }
            None => {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(WebhookResponse {
                        message_id: Uuid::nil(),
                        status: "error".to_string(),
                        response: Some("Webhook secret required".to_string()),
                    }),
                );
            }
        }
    }

    if req.content.len() > MAX_CONTENT_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(WebhookResponse {
                message_id: Uuid::nil(),
                status: "error".to_string(),
                response: Some("Content too large".to_string()),
            }),
        );
    }

    let msg = IncomingMessage::new("http", &state.user_id, &req.content).with_metadata(
        serde_json::json!({
            "wait_for_response": req.wait_for_response,
        }),
    );

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
) -> (StatusCode, Json<WebhookResponse>) {
    let msg_id = msg.id;

    // Set up response channel if waiting
    let response_rx = if wait_for_response {
        if state.pending_responses.read().await.len() >= MAX_PENDING_RESPONSES {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(WebhookResponse {
                    message_id: msg_id,
                    status: "error".to_string(),
                    response: Some("Too many pending requests".to_string()),
                }),
            );
        }

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

    // Ensure pending response entry is cleaned up on timeout or cancellation
    let _ = state.pending_responses.write().await.remove(&msg_id);

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
        if self.state.webhook_secret.is_none() {
            return Err(ChannelError::StartupFailed {
                name: "http".to_string(),
                reason: "HTTP webhook secret is required (set HTTP_WEBHOOK_SECRET)".to_string(),
            });
        }

        let (tx, rx) = mpsc::channel(256);
        *self.state.tx.write().await = Some(tx);

        tracing::info!(
            "HTTP channel ready ({}:{})",
            self.config.host,
            self.config.port
        );

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
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        if self.state.tx.read().await.is_some() {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: "http".to_string(),
            })
        }
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        *self.state.tx.write().await = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_http_channel_requires_secret() {
        let config = HttpConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            webhook_secret: None,
            user_id: "http".to_string(),
        };

        let channel = HttpChannel::new(config);
        let result = channel.start().await;
        assert!(result.is_err());
    }
}
