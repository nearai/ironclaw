//! HTTP webhook channel for receiving messages via HTTP POST.

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use bytes::Bytes;
use hmac::{Hmac, Mac};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use crate::channels::{
    AttachmentKind, Channel, ChannelSecretUpdater, IncomingAttachment, IncomingMessage,
    MessageStream, OutgoingResponse,
};
use crate::config::HttpConfig;
use crate::error::ChannelError;

type HmacSha256 = Hmac<Sha256>;

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
    /// Optional caller or client identifier for sender-scoped routing.
    /// The channel owner/storage scope remains fixed by server config.
    #[serde(default)]
    user_id: Option<String>,
    /// Message content.
    content: String,
    /// Optional thread ID for conversation tracking.
    thread_id: Option<String>,
    /// Deprecated: webhook secret in request body. Use X-Hub-Signature-256 header instead.
    /// This field is accepted for backward compatibility but will be removed in a future release.
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

/// Verify an HMAC-SHA256 signature against the raw request body.
///
/// The expected header format is: `sha256=<hex_digest>`
/// where the digest is HMAC-SHA256(secret_key, body_bytes) encoded as lowercase hex.
fn verify_hmac_signature(secret: &str, body: &[u8], signature_header: &str) -> bool {
    let hex_digest = match signature_header.strip_prefix("sha256=") {
        Some(h) => h,
        None => return false,
    };

    let provided_mac = match hex::decode(hex_digest) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };

    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(mac) => mac,
        Err(_) => return false,
    };
    mac.update(body);
    let expected_mac = mac.finalize().into_bytes();

    bool::from(expected_mac.as_slice().ct_eq(&provided_mac))
}

async fn webhook_handler(
    State(state): State<Arc<HttpChannelState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
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
            )
                .into_response();
        }
    }

    let content_type_ok = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.starts_with("application/json"))
        .unwrap_or(false);

    if !content_type_ok {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(WebhookResponse {
                message_id: Uuid::nil(),
                status: "error".to_string(),
                response: Some("Content-Type must be application/json".to_string()),
            }),
        )
            .into_response();
    }

    let mut fallback_req = None;
    {
        let webhook_secret = state.webhook_secret.read().await;
        let expected_secret = match webhook_secret.as_ref() {
            Some(secret) => secret.expose_secret(),
            None => {
                // No secret configured — reject all requests. This guards against
                // the secret being cleared at runtime via update_secret(None).
                // The start() method also prevents startup without a secret, but
                // this is defense-in-depth for the SIGHUP hot-swap path.
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(WebhookResponse {
                        message_id: Uuid::nil(),
                        status: "error".to_string(),
                        response: Some("Webhook authentication not configured".to_string()),
                    }),
                )
                    .into_response();
            }
        };

        match headers.get("x-hub-signature-256") {
            Some(raw_signature) => match raw_signature.to_str() {
                Ok(signature) => {
                    if !verify_hmac_signature(expected_secret, &body, signature) {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(WebhookResponse {
                                message_id: Uuid::nil(),
                                status: "error".to_string(),
                                response: Some("Invalid webhook signature".to_string()),
                            }),
                        )
                            .into_response();
                    }
                }
                Err(_) => {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(WebhookResponse {
                            message_id: Uuid::nil(),
                            status: "error".to_string(),
                            response: Some("Invalid signature header encoding".to_string()),
                        }),
                    )
                        .into_response();
                }
            },
            None => {
                let req: WebhookRequest = match serde_json::from_slice(&body) {
                    Ok(req) => req,
                    Err(_) => {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(WebhookResponse {
                                message_id: Uuid::nil(),
                                status: "error".to_string(),
                                response: Some(
                                    "Webhook authentication required. Provide X-Hub-Signature-256 header \
                                     (preferred) or 'secret' field in body (deprecated)."
                                        .to_string(),
                                ),
                            }),
                        )
                            .into_response();
                    }
                };

                match &req.secret {
                    Some(provided)
                        if bool::from(provided.as_bytes().ct_eq(expected_secret.as_bytes())) =>
                    {
                        tracing::warn!(
                            "Webhook authenticated via deprecated 'secret' field in request body. \
                             Migrate to X-Hub-Signature-256 header (HMAC-SHA256). \
                             Body secret support will be removed in a future release."
                        );
                        fallback_req = Some(req);
                    }
                    Some(_) => {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(WebhookResponse {
                                message_id: Uuid::nil(),
                                status: "error".to_string(),
                                response: Some("Invalid webhook secret".to_string()),
                            }),
                        )
                            .into_response();
                    }
                    None => {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(WebhookResponse {
                                message_id: Uuid::nil(),
                                status: "error".to_string(),
                                response: Some(
                                    "Webhook authentication required. Provide X-Hub-Signature-256 header \
                                     (preferred) or 'secret' field in body (deprecated)."
                                        .to_string(),
                                ),
                            }),
                        )
                            .into_response();
                    }
                }
            }
        }
    }

    if let Some(req) = fallback_req {
        return process_authenticated_request(state, req).await;
    }

    let req: WebhookRequest = match serde_json::from_slice(&body) {
        Ok(req) => req,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(WebhookResponse {
                    message_id: Uuid::nil(),
                    status: "error".to_string(),
                    response: Some(format!("Invalid JSON: {e}")),
                }),
            )
                .into_response();
        }
    };

    process_authenticated_request(state, req).await
}

async fn process_authenticated_request(
    state: Arc<HttpChannelState>,
    req: WebhookRequest,
) -> axum::response::Response {
    let _ = req.user_id.as_ref().map(|user_id| {
        tracing::debug!(
            provided_user_id = %user_id,
            configured_owner_id = %state.user_id,
            "HTTP webhook request provided user_id; using it as sender_id while keeping the configured owner scope"
        );
    });

    if req.content.len() > MAX_CONTENT_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(WebhookResponse {
                message_id: Uuid::nil(),
                status: "error".to_string(),
                response: Some("Content too large".to_string()),
            }),
        )
            .into_response();
    }

    let wait_for_response = req.wait_for_response;

    let attachments = if !req.attachments.is_empty() {
        if req.attachments.len() > MAX_ATTACHMENTS {
            return (
                StatusCode::BAD_REQUEST,
                Json(WebhookResponse {
                    message_id: Uuid::nil(),
                    status: "error".to_string(),
                    response: Some(format!("Too many attachments (max {})", MAX_ATTACHMENTS)),
                }),
            )
                .into_response();
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
                        )
                            .into_response();
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
                    )
                        .into_response();
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
                    )
                        .into_response();
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

    let sender_id = req.user_id.clone().unwrap_or_else(|| state.user_id.clone());
    let mut msg = IncomingMessage::new("http", &state.user_id, &req.content)
        .with_owner_id(&state.user_id)
        .with_sender_id(sender_id)
        .with_metadata(serde_json::json!({
            "wait_for_response": wait_for_response,
        }));

    if !attachments.is_empty() {
        msg = msg.with_attachments(attachments);
    }

    if let Some(thread_id) = &req.thread_id {
        msg = msg.with_thread(thread_id);
    }

    process_message(state, msg, wait_for_response)
        .await
        .into_response()
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

    // Clone sender while holding read lock, then release lock before async send.
    // This prevents blocking other webhook handlers during the async I/O.
    let tx = {
        let guard = state.tx.read().await;
        guard.as_ref().cloned()
    };

    if let Some(tx) = tx {
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

/// Implement secret update for HTTP channel state.
/// This allows SIGHUP handler to update secrets generically via the trait.
#[async_trait]
impl ChannelSecretUpdater for HttpChannelState {
    async fn update_secret(&self, new_secret: Option<SecretString>) {
        *self.webhook_secret.write().await = new_secret;
        tracing::info!("HTTP webhook secret updated");
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{HeaderValue, Request};
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

    fn compute_signature(secret: &str, body: &[u8]) -> String {
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC key creation failed");
        mac.update(body);
        let result = mac.finalize().into_bytes();
        format!("sha256={}", hex::encode(result))
    }

    #[tokio::test]
    async fn test_http_channel_requires_secret() {
        let channel = test_channel(None);
        let result = channel.start().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn webhook_hmac_signature_returns_ok() {
        let secret = "test-secret-123";
        let channel = test_channel(Some(secret));
        let _stream = channel.start().await.unwrap();
        let app = channel.routes();

        let body = serde_json::json!({
            "content": "hello"
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let signature = compute_signature(secret, &body_bytes);
        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", signature)
            .body(Body::from(body_bytes))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn webhook_wrong_hmac_signature_returns_unauthorized() {
        let channel = test_channel(Some("correct-secret"));
        let _stream = channel.start().await.unwrap();
        let app = channel.routes();

        let body = serde_json::json!({
            "content": "hello"
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let signature = compute_signature("wrong-secret", &body_bytes);
        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", signature)
            .body(Body::from(body_bytes))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn webhook_malformed_signature_returns_unauthorized() {
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
            .header("x-hub-signature-256", "not-a-valid-signature")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn webhook_deprecated_body_secret_still_works() {
        let channel = test_channel(Some("test-secret-123"));
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
    async fn webhook_wrong_body_secret_returns_unauthorized() {
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

    /// Regression test for issue #869: RwLock read guard was held across
    /// tx.send(msg).await in `process_message()`, blocking shutdown() from
    /// acquiring the write lock when the channel buffer was full.
    ///
    /// This test exercises the actual production code path (`process_message`)
    /// with a full channel buffer, then verifies shutdown() can still complete.
    #[tokio::test]
    async fn shutdown_completes_while_process_message_blocked() {
        let channel = Arc::new(test_channel(Some("secret")));
        let stream = channel.start().await.unwrap();

        // Fill all 256 slots in the channel buffer
        {
            let tx = {
                let guard = channel.state.tx.read().await;
                guard.as_ref().unwrap().clone()
            };
            for i in 0..256 {
                let msg = IncomingMessage::new("http", "user", format!("fill-{}", i));
                tx.send(msg).await.unwrap();
            }
        }

        // Signal so we know the spawned task has started and is about to
        // call process_message (which will block on the full channel).
        let started = Arc::new(tokio::sync::Notify::new());
        let started_clone = started.clone();

        // Spawn a task that calls the actual production code path.
        // process_message() internally acquires the RwLock read guard and
        // sends on the channel. With the fix, the guard is released before
        // send().await; without the fix, shutdown() would deadlock.
        let state = channel.state.clone();
        let blocked_send = tokio::spawn(async move {
            started_clone.notify_one();
            let msg = IncomingMessage::new("http", "user", "blocked-257th");
            let _ = process_message(state, msg, false).await;
        });

        // Wait for the spawned task to start, then give it time to reach
        // the send().await and verify that it is still pending (i.e., blocked).
        started.notified().await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !blocked_send.is_finished(),
            "process_message task should still be pending before shutdown()"
        );

        // shutdown() must complete even though process_message is blocked on
        // send(). Before the fix, the read guard held across send().await
        // would prevent shutdown() from acquiring the write lock.
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(2), channel.shutdown()).await;
        assert!(result.is_ok(), "shutdown() must not deadlock");
        assert!(result.unwrap().is_ok());

        // Drop the stream (receiver) so the blocked send task can complete
        drop(stream);
        let _ = blocked_send.await;
    }

    #[tokio::test]
    async fn webhook_missing_all_auth_returns_unauthorized() {
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
    async fn webhook_hmac_takes_precedence_over_body_secret() {
        let secret = "test-secret-123";
        let channel = test_channel(Some(secret));
        let _stream = channel.start().await.unwrap();
        let app = channel.routes();

        let body = serde_json::json!({
            "content": "hello",
            "secret": "wrong-secret-in-body"
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let signature = compute_signature(secret, &body_bytes);

        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", signature)
            .body(Body::from(body_bytes))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn webhook_invalid_json_returns_bad_request() {
        let secret = "test-secret";
        let channel = test_channel(Some(secret));
        let _stream = channel.start().await.unwrap();
        let app = channel.routes();

        let body = b"not json".to_vec();
        let signature = compute_signature(secret, &body);

        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", signature)
            .body(Body::from(body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn webhook_rejects_non_json_content_type() {
        let secret = "test-secret";
        let channel = test_channel(Some(secret));
        let _stream = channel.start().await.unwrap();
        let app = channel.routes();

        let body = serde_json::json!({
            "content": "hello"
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let signature = compute_signature(secret, &body_bytes);

        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "text/plain")
            .header("x-hub-signature-256", signature)
            .body(Body::from(body_bytes))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn webhook_invalid_signature_header_encoding_returns_unauthorized() {
        let channel = test_channel(Some("test-secret"));
        let _stream = channel.start().await.unwrap();
        let app = channel.routes();

        let body = serde_json::json!({
            "content": "hello"
        });

        let mut req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        req.headers_mut().insert(
            "x-hub-signature-256",
            HeaderValue::from_bytes(b"\xFF").unwrap(),
        );

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
    async fn webhook_rejects_requests_after_secret_is_cleared() {
        let secret = "test-secret-123";
        let channel = test_channel(Some(secret));
        let _stream = channel.start().await.unwrap();
        let app = channel.routes();

        channel.update_secret(None).await;

        let body = serde_json::json!({
            "content": "hello"
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let signature = compute_signature(secret, &body_bytes);
        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", signature)
            .body(Body::from(body_bytes))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE); // safety: test assertion
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

    #[test]
    fn verify_hmac_signature_valid() {
        let secret = "my-secret";
        let body = b"test body content";
        let sig = compute_signature(secret, body);
        assert!(verify_hmac_signature(secret, body, &sig));
    }

    #[test]
    fn verify_hmac_signature_invalid_digest() {
        let secret = "my-secret";
        let body = b"test body content";
        assert!(!verify_hmac_signature(
            secret,
            body,
            "sha256=0000000000000000000000000000000000000000000000000000000000000000"
        ));
    }

    #[test]
    fn verify_hmac_signature_missing_prefix() {
        let secret = "my-secret";
        let body = b"test body content";
        assert!(!verify_hmac_signature(secret, body, "deadbeef"));
    }

    #[test]
    fn verify_hmac_signature_invalid_hex() {
        let secret = "my-secret";
        let body = b"test body content";
        assert!(!verify_hmac_signature(secret, body, "sha256=not-hex!"));
    }

    /// Regression test for issue #1033: when the webhook secret is cleared at
    /// runtime via update_secret(None), subsequent requests must be rejected
    /// instead of being processed without authentication.
    #[tokio::test]
    async fn webhook_rejects_when_secret_cleared_at_runtime() {
        let channel = test_channel(Some("initial-secret"));
        let _stream = channel.start().await.unwrap();

        // Clear the secret at runtime (simulates a bad SIGHUP config reload)
        channel.update_secret(None).await;

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
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "requests must be rejected when webhook secret is cleared at runtime"
        );
    }
}
