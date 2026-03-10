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
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use crate::channels::{
    AttachmentKind, Channel, IncomingAttachment, IncomingMessage, MessageStream, OutgoingResponse,
};
use crate::config::HttpConfig;
use crate::error::ChannelError;

/// HTTP webhook channel.
pub struct HttpChannel {
    config: HttpConfig,
    state: Arc<HttpChannelState>,
}

pub struct HttpChannelState {
    /// Sender for incoming messages.
    tx: RwLock<Option<mpsc::Sender<IncomingMessage>>>,
    /// Pending responses keyed by message ID.
    pending_responses: RwLock<std::collections::HashMap<Uuid, oneshot::Sender<String>>>,
    /// Expected webhook secret for authentication (if configured).
    /// Stored in a separate Arc<RwLock<>> to avoid contending with other state operations.
    /// Rarely changes (only on SIGHUP), so isolated from hot-path state accesses.
    /// Uses SecretString to prevent accidental logging and memory dump exposure.
    webhook_secret: Arc<RwLock<Option<SecretString>>>,
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

impl HttpChannelState {
    /// Update the webhook secret in-place without restarting the listener.
    /// Called during SIGHUP to hot-swap credentials.
    pub async fn update_secret(&self, new_secret: Option<SecretString>) {
        *self.webhook_secret.write().await = new_secret;
    }
}

/// Maximum JSON body size for webhook requests (15 MB, to support base64 image attachments
/// with ~33% overhead from base64 encoding).
const MAX_BODY_BYTES: usize = 15 * 1024 * 1024;

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
            .map(|s| SecretString::from(s.expose_secret().to_string()));
        let user_id = config.user_id.clone();

        Self {
            config,
            state: Arc::new(HttpChannelState {
                tx: RwLock::new(None),
                pending_responses: RwLock::new(std::collections::HashMap::new()),
                webhook_secret: Arc::new(RwLock::new(webhook_secret)),
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

    /// Return a shared handle to the channel state for out-of-band updates.
    pub fn shared_state(&self) -> Arc<HttpChannelState> {
        Arc::clone(&self.state)
    }

    /// Update the webhook secret in-place without restarting the listener.
    pub async fn update_secret(&self, new_secret: Option<SecretString>) {
        self.state.update_secret(new_secret).await;
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
    /// Optional file attachments (base64-encoded).
    #[serde(default)]
    attachments: Vec<AttachmentData>,
}

/// A file attachment in a webhook request.
#[derive(Debug, Deserialize)]
struct AttachmentData {
    /// MIME type (e.g. "image/png", "application/pdf").
    mime_type: String,
    /// Optional filename.
    #[serde(default)]
    filename: Option<String>,
    /// Base64-encoded file data.
    #[serde(default)]
    data_base64: Option<String>,
    /// URL to fetch the file from (not downloaded server-side for SSRF prevention).
    #[serde(default)]
    url: Option<String>,
}

/// Maximum size per attachment (5 MB decoded).
const MAX_ATTACHMENT_BYTES: usize = 5 * 1024 * 1024;
/// Maximum total attachment size (10 MB decoded).
const MAX_TOTAL_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
/// Maximum number of attachments per request.
const MAX_ATTACHMENTS: usize = 5;

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
    if let Some(ref expected_secret) = *state.webhook_secret.read().await {
        let expected_bytes = expected_secret.expose_secret().as_bytes();
        match &req.secret {
            Some(provided) if bool::from(provided.as_bytes().ct_eq(expected_bytes)) => {
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

    // Validate and decode attachments
    let attachments = if !req.attachments.is_empty() {
        if req.attachments.len() > MAX_ATTACHMENTS {
            return (
                StatusCode::BAD_REQUEST,
                Json(WebhookResponse {
                    message_id: Uuid::nil(),
                    status: "error".to_string(),
                    response: Some(format!("Too many attachments (max {})", MAX_ATTACHMENTS)),
                }),
            );
        }

        let mut decoded_attachments = Vec::new();
        let mut total_bytes: usize = 0;
        for att in &req.attachments {
            if let Some(ref b64) = att.data_base64 {
                use base64::Engine;
                let data = match base64::engine::general_purpose::STANDARD.decode(b64) {
                    Ok(d) => d,
                    Err(_) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(WebhookResponse {
                                message_id: Uuid::nil(),
                                status: "error".to_string(),
                                response: Some("Invalid base64 in attachment".to_string()),
                            }),
                        );
                    }
                };
                if data.len() > MAX_ATTACHMENT_BYTES {
                    return (
                        StatusCode::PAYLOAD_TOO_LARGE,
                        Json(WebhookResponse {
                            message_id: Uuid::nil(),
                            status: "error".to_string(),
                            response: Some(format!(
                                "Attachment too large (max {} bytes)",
                                MAX_ATTACHMENT_BYTES
                            )),
                        }),
                    );
                }
                total_bytes += data.len();
                if total_bytes > MAX_TOTAL_ATTACHMENT_BYTES {
                    return (
                        StatusCode::PAYLOAD_TOO_LARGE,
                        Json(WebhookResponse {
                            message_id: Uuid::nil(),
                            status: "error".to_string(),
                            response: Some("Total attachment size exceeds limit".to_string()),
                        }),
                    );
                }
                decoded_attachments.push(IncomingAttachment {
                    id: Uuid::new_v4().to_string(),
                    kind: AttachmentKind::from_mime_type(&att.mime_type),
                    mime_type: att.mime_type.clone(),
                    filename: att.filename.clone(),
                    size_bytes: Some(data.len() as u64),
                    source_url: None,
                    storage_key: None,
                    extracted_text: None,
                    data,
                    duration_secs: None,
                });
            } else if let Some(ref url) = att.url {
                // URL-only attachment: set source_url but don't download (SSRF prevention)
                decoded_attachments.push(IncomingAttachment {
                    id: Uuid::new_v4().to_string(),
                    kind: AttachmentKind::from_mime_type(&att.mime_type),
                    mime_type: att.mime_type.clone(),
                    filename: att.filename.clone(),
                    size_bytes: None,
                    source_url: Some(url.clone()),
                    storage_key: None,
                    extracted_text: None,
                    data: Vec::new(),
                    duration_secs: None,
                });
            }
        }
        decoded_attachments
    } else {
        Vec::new()
    };

    let mut msg = IncomingMessage::new("http", &state.user_id, &req.content).with_metadata(
        serde_json::json!({
            "wait_for_response": req.wait_for_response,
        }),
    );

    if !attachments.is_empty() {
        msg = msg.with_attachments(attachments);
    }

    if let Some(thread_id) = &req.thread_id {
        msg = msg.with_thread(thread_id);
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
        if self.state.webhook_secret.read().await.is_none() {
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
    use axum::body::Body;
    use axum::http::Request;
    use secrecy::SecretString;
    use tower::ServiceExt;

    use super::*;

    fn test_channel(secret: Option<&str>) -> HttpChannel {
        HttpChannel::new(HttpConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            webhook_secret: secret.map(|s| SecretString::from(s.to_string())),
            user_id: "http".to_string(),
        })
    }

    #[tokio::test]
    async fn test_http_channel_requires_secret() {
        let channel = test_channel(None);
        let result = channel.start().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn webhook_correct_secret_returns_ok() {
        let channel = test_channel(Some("test-secret-123"));
        // Start the channel so the tx sender is populated (otherwise 503).
        let _stream = channel.start().await.unwrap();
        let app = channel.routes();

        let body = serde_json::json!({
            "content": "hello",
            "secret": "test-secret-123"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn webhook_wrong_secret_returns_unauthorized() {
        let channel = test_channel(Some("correct-secret"));
        let _stream = channel.start().await.unwrap();
        let app = channel.routes();

        let body = serde_json::json!({
            "content": "hello",
            "secret": "wrong-secret"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn webhook_missing_secret_returns_unauthorized() {
        let channel = test_channel(Some("correct-secret"));
        let _stream = channel.start().await.unwrap();
        let app = channel.routes();

        let body = serde_json::json!({
            "content": "hello"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_update_secret_hot_swap() {
        let channel = test_channel(Some("old-secret"));
        let _stream = channel.start().await.unwrap();
        let app1 = channel.routes();

        // Request with old-secret should succeed
        let body_old = serde_json::json!({
            "content": "hello",
            "secret": "old-secret"
        });
        let req1 = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body_old).unwrap()))
            .unwrap();
        let resp1 = app1.oneshot(req1).await.unwrap();
        assert_eq!(
            resp1.status(),
            StatusCode::OK,
            "old secret should work initially"
        );

        // Update secret to new-secret
        channel
            .update_secret(Some(SecretString::from("new-secret".to_string())))
            .await;

        let app2 = channel.routes();

        // Request with old-secret should fail
        let req2 = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body_old).unwrap()))
            .unwrap();
        let resp2 = app2.oneshot(req2).await.unwrap();
        assert_eq!(
            resp2.status(),
            StatusCode::UNAUTHORIZED,
            "old secret should fail after update"
        );

        let app3 = channel.routes();

        // Request with new-secret should succeed
        let body_new = serde_json::json!({
            "content": "hello",
            "secret": "new-secret"
        });
        let req3 = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body_new).unwrap()))
            .unwrap();
        let resp3 = app3.oneshot(req3).await.unwrap();
        assert_eq!(
            resp3.status(),
            StatusCode::OK,
            "new secret should work after update"
        );
    }

    #[tokio::test]
    async fn test_concurrent_requests_during_secret_update() {
        use std::sync::Arc as StdArc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        let channel = test_channel(Some("initial-secret"));
        let _stream = channel.start().await.unwrap();
        let app = channel.routes();

        // Counters for request outcomes
        let success_count = StdArc::new(AtomicUsize::new(0));

        let mut handles = vec![];

        // Spawn 5 concurrent tasks that keep making requests with the initial secret
        for i in 0..5 {
            let app = app.clone();
            let success = StdArc::clone(&success_count);

            let handle = tokio::spawn(async move {
                let body = serde_json::json!({
                    "content": format!("test-{}", i),
                    "secret": "initial-secret"
                });

                let req = Request::builder()
                    .method("POST")
                    .uri("/webhook")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap();

                let resp = app.oneshot(req).await.unwrap();
                if resp.status() == StatusCode::OK {
                    success.fetch_add(1, Ordering::SeqCst);
                }
            });
            handles.push(handle);
        }

        // Update secret mid-flight (tests that RwLock allows readers while writer holds lock)
        tokio::time::sleep(Duration::from_millis(5)).await;
        channel
            .update_secret(Some(SecretString::from("updated-secret".to_string())))
            .await;

        // Spawn 5 more tasks that use the new secret
        for i in 5..10 {
            let app = app.clone();
            let success = StdArc::clone(&success_count);

            let handle = tokio::spawn(async move {
                let body = serde_json::json!({
                    "content": format!("test-{}", i),
                    "secret": "updated-secret"
                });

                let req = Request::builder()
                    .method("POST")
                    .uri("/webhook")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap();

                let resp = app.oneshot(req).await.unwrap();
                if resp.status() == StatusCode::OK {
                    success.fetch_add(1, Ordering::SeqCst);
                }
            });
            handles.push(handle);
        }

        // Wait for all tasks to complete
        for handle in handles {
            let _ = handle.await;
        }

        // Verify all requests succeeded with their respective secrets
        assert_eq!(
            success_count.load(Ordering::SeqCst),
            10,
            "All concurrent requests should succeed with correct secrets after update"
        );
    }
}
