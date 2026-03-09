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
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use crate::channels::{
    AttachmentKind, Channel, IncomingAttachment, IncomingMessage, MessageStream, OutgoingResponse,
};
use crate::config::HttpConfig;
use crate::error::ChannelError;

type HmacSha256 = Hmac<Sha256>;

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
    /// Deprecated: webhook secret in request body. Use X-IronClaw-Signature header instead.
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
    // The header must start with "sha256="
    let hex_digest = match signature_header.strip_prefix("sha256=") {
        Some(h) => h,
        None => return false,
    };

    // Decode the provided hex digest
    let provided_mac = match hex::decode(hex_digest) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };

    // Compute expected HMAC
    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    let expected_mac = mac.finalize().into_bytes();

    // Constant-time comparison to prevent timing attacks
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

    // Content-Type validation: reject non-JSON payloads before any processing
    let content_type_ok = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.starts_with("application/json"))
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

    // Authenticate BEFORE parsing JSON to avoid leaking parse-validity to
    // unauthenticated callers (they would otherwise see 400 vs 401 differences).
    if let Some(ref expected_secret) = state.webhook_secret {
        match headers.get("x-ironclaw-signature") {
            Some(raw) => {
                // Signature header is present -- verify it
                match raw.to_str() {
                    Ok(sig) => {
                        if !verify_hmac_signature(expected_secret, &body, sig) {
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
                        // Header present but not valid UTF-8 -- reject immediately
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
                }
            }
            None => {
                // No signature header -- fall back to deprecated body secret.
                // We must parse JSON here to extract the secret field, but this
                // is acceptable because the absence of the header is itself a
                // distinguishable state (the caller chose the deprecated path).
                let req: WebhookRequest = match serde_json::from_slice(&body) {
                    Ok(r) => r,
                    Err(_) => {
                        // Return 401 (not 400) so unauthenticated callers cannot
                        // distinguish parse failures from auth failures.
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(WebhookResponse {
                                message_id: Uuid::nil(),
                                status: "error".to_string(),
                                response: Some(
                                    "Webhook authentication required. Provide X-IronClaw-Signature header \
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
                             Migrate to X-IronClaw-Signature header (HMAC-SHA256). \
                             Body secret support will be removed in a future release."
                        );
                        // Authenticated via deprecated path -- continue with already-parsed request
                        return process_authenticated_request(state, req).await;
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
                                    "Webhook authentication required. Provide X-IronClaw-Signature header \
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

    // Authentication passed (or no secret configured) -- now parse JSON
    let req: WebhookRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
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

/// Process a request that has already been authenticated and parsed.
async fn process_authenticated_request(
    state: Arc<HttpChannelState>,
    req: WebhookRequest,
) -> axum::response::Response {
    if let Some(ref user_id) = req.user_id {
        tracing::debug!(
            provided_user_id = %user_id,
            "HTTP webhook request provided user_id, ignoring in favor of configured user_id"
        );
    }

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

    /// Compute an HMAC-SHA256 signature for a body using the given secret,
    /// returning the `sha256=<hex>` formatted string suitable for the header.
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

        let body_json = serde_json::json!({
            "content": "hello"
        });
        let body_bytes = serde_json::to_vec(&body_json).unwrap();
        let signature = compute_signature(secret, &body_bytes);

        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .header("x-ironclaw-signature", &signature)
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

        let body_json = serde_json::json!({
            "content": "hello"
        });
        let body_bytes = serde_json::to_vec(&body_json).unwrap();
        // Sign with the wrong secret
        let signature = compute_signature("wrong-secret", &body_bytes);

        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .header("x-ironclaw-signature", &signature)
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

        let body_json = serde_json::json!({
            "content": "hello"
        });
        let body_bytes = serde_json::to_vec(&body_json).unwrap();

        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .header("x-ironclaw-signature", "not-a-valid-signature")
            .body(Body::from(body_bytes))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn webhook_deprecated_body_secret_still_works() {
        // Backward compatibility: old-style secret in body should still authenticate
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
        // If both header and body secret are present, HMAC header is used
        let secret = "test-secret-123";
        let channel = test_channel(Some(secret));
        let _stream = channel.start().await.unwrap();
        let app = channel.routes();

        let body_json = serde_json::json!({
            "content": "hello",
            "secret": "wrong-secret-in-body"
        });
        let body_bytes = serde_json::to_vec(&body_json).unwrap();
        // Sign with the correct secret via header
        let signature = compute_signature(secret, &body_bytes);

        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .header("x-ironclaw-signature", &signature)
            .body(Body::from(body_bytes))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Should succeed because HMAC header is valid, body secret is ignored
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn webhook_invalid_json_returns_bad_request() {
        let secret = "test-secret";
        let channel = test_channel(Some(secret));
        let _stream = channel.start().await.unwrap();
        let app = channel.routes();

        let body_bytes = b"not json";
        // Compute a VALID HMAC over the invalid body so auth passes,
        // then the JSON parse fails and we get 400.
        let signature = compute_signature(secret, body_bytes);

        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .header("x-ironclaw-signature", &signature)
            .body(Body::from(body_bytes.to_vec()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn webhook_missing_content_type_returns_415() {
        let secret = "test-secret";
        let channel = test_channel(Some(secret));
        let _stream = channel.start().await.unwrap();
        let app = channel.routes();

        let body_json = serde_json::json!({ "content": "hello" });
        let body_bytes = serde_json::to_vec(&body_json).unwrap();
        let signature = compute_signature(secret, &body_bytes);

        // No content-type header
        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("x-ironclaw-signature", &signature)
            .body(Body::from(body_bytes))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn webhook_wrong_content_type_returns_415() {
        let secret = "test-secret";
        let channel = test_channel(Some(secret));
        let _stream = channel.start().await.unwrap();
        let app = channel.routes();

        let body_bytes = b"some text data";
        let signature = compute_signature(secret, body_bytes);

        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "text/plain")
            .header("x-ironclaw-signature", &signature)
            .body(Body::from(body_bytes.to_vec()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn webhook_non_utf8_signature_header_returns_unauthorized() {
        let channel = test_channel(Some("test-secret"));
        let _stream = channel.start().await.unwrap();
        let app = channel.routes();

        let body_json = serde_json::json!({ "content": "hello" });
        let body_bytes = serde_json::to_vec(&body_json).unwrap();

        // Construct a request with a non-UTF-8 header value
        let req = Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .header(
                "x-ironclaw-signature",
                axum::http::HeaderValue::from_bytes(b"\xff\xfe").unwrap(),
            )
            .body(Body::from(body_bytes))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
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
}
