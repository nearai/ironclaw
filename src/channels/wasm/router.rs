//! HTTP router for WASM channel webhooks.
//!
//! Routes incoming HTTP requests to the appropriate WASM channel based on
//! registered paths. Handles secret validation at the host level.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, oneshot};

use crate::channels::wasm::wrapper::{HttpResponseWithMessages, WasmChannel};

/// Metadata extracted from WhatsApp webhook messages.
/// Used for ACK keys, deduplication, and mark_as_read API calls.
#[derive(Deserialize)]
struct WhatsAppMetadata {
    /// The WhatsApp message ID (wamid.xxx) used for ACK and deduplication.
    message_id: Option<String>,
    /// The phone number ID for the WhatsApp Business account.
    phone_number_id: Option<String>,
}

/// A registered HTTP endpoint for a WASM channel.
#[derive(Debug, Clone)]
pub struct RegisteredEndpoint {
    /// Channel name that owns this endpoint.
    pub channel_name: String,
    /// HTTP path (e.g., "/webhook/slack").
    pub path: String,
    /// Allowed HTTP methods.
    pub methods: Vec<String>,
    /// Whether secret validation is required.
    pub require_secret: bool,
}

/// Router for WASM channel HTTP endpoints.
pub struct WasmChannelRouter {
    /// Registered channels by name.
    channels: RwLock<HashMap<String, Arc<WasmChannel>>>,
    /// Path to channel mapping for fast lookup.
    path_to_channel: RwLock<HashMap<String, String>>,
    /// Expected webhook secrets by channel name.
    secrets: RwLock<HashMap<String, String>>,
    /// Webhook secret header names by channel name (e.g., "X-Telegram-Bot-Api-Secret-Token").
    secret_headers: RwLock<HashMap<String, String>>,
    /// Ed25519 public keys for signature verification by channel name (hex-encoded).
    signature_keys: RwLock<HashMap<String, String>>,
    /// Verification mode per channel: "query_param", "signature", etc.
    verification_modes: RwLock<HashMap<String, String>>,
    /// HMAC secrets for signature verification by channel name (WhatsApp/Slack style).
    hmac_secrets: RwLock<HashMap<String, String>>,
    /// Pending webhook acknowledgments by "channel:message_id" key.
    /// Used to delay webhook 200 response until message is persisted to DB.
    pending_acks: RwLock<HashMap<String, oneshot::Sender<()>>>,
    /// Access tokens by channel name (for host-side API calls like mark_as_read).
    access_tokens: RwLock<HashMap<String, String>>,
    /// API versions by channel name (for host-side API calls).
    api_versions: RwLock<HashMap<String, String>>,
    /// Database for webhook message deduplication.
    db: RwLock<Option<Arc<dyn crate::db::WebhookDedupStore + Send + Sync>>>,
}

impl WasmChannelRouter {
    /// Create a new router.
    pub fn new() -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
            path_to_channel: RwLock::new(HashMap::new()),
            secrets: RwLock::new(HashMap::new()),
            secret_headers: RwLock::new(HashMap::new()),
            signature_keys: RwLock::new(HashMap::new()),
            verification_modes: RwLock::new(HashMap::new()),
            hmac_secrets: RwLock::new(HashMap::new()),
            pending_acks: RwLock::new(HashMap::new()),
            access_tokens: RwLock::new(HashMap::new()),
            api_versions: RwLock::new(HashMap::new()),
            db: RwLock::new(None),
        }
    }

    /// Set the database for webhook message deduplication.
    pub async fn set_db(&self, db: Arc<dyn crate::db::WebhookDedupStore + Send + Sync>) {
        *self.db.write().await = Some(db);
    }

    /// Get the database for webhook message deduplication.
    pub async fn get_db(&self) -> Option<Arc<dyn crate::db::WebhookDedupStore + Send + Sync>> {
        self.db.read().await.clone()
    }

    /// Clean up old webhook dedup records.
    ///
    /// Called periodically to prevent unbounded growth of the dedup table.
    /// Returns the number of records deleted.
    pub async fn cleanup_old_dedup_records(&self) -> usize {
        if let Some(db) = self.get_db().await {
            match db.cleanup_old_webhook_dedup_records().await {
                Ok(count) => {
                    if count > 0 {
                        tracing::info!(
                            deleted_count = count,
                            "Cleaned up old webhook dedup records"
                        );
                    }
                    count as usize
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "Failed to clean up old webhook dedup records"
                    );
                    0
                }
            }
        } else {
            0
        }
    }

    /// Register a channel with its endpoints.
    ///
    /// # Arguments
    /// * `channel` - The WASM channel to register
    /// * `endpoints` - HTTP endpoints to register for this channel
    /// * `secret` - Optional webhook secret for validation
    /// * `secret_header` - Optional HTTP header name for secret validation
    ///   (e.g., "X-Telegram-Bot-Api-Secret-Token"). Defaults to "X-Webhook-Secret".
    /// * `verification_mode` - Optional verification mode for GET requests:
    ///   - "query_param": Skip host-level secret validation for GET, WASM validates via query param
    ///   - "signature": Always require signature validation
    pub async fn register(
        &self,
        channel: Arc<WasmChannel>,
        endpoints: Vec<RegisteredEndpoint>,
        secret: Option<String>,
        secret_header: Option<String>,
        verification_mode: Option<String>,
    ) {
        let name = channel.channel_name().to_string();

        // Store the channel
        self.channels.write().await.insert(name.clone(), channel);

        // Register path mappings
        let mut path_map = self.path_to_channel.write().await;
        for endpoint in endpoints {
            path_map.insert(endpoint.path.clone(), name.clone());
            tracing::info!(
                channel = %name,
                path = %endpoint.path,
                methods = ?endpoint.methods,
                "Registered WASM channel HTTP endpoint"
            );
        }

        // Store secret if provided
        if let Some(s) = secret {
            self.secrets.write().await.insert(name.clone(), s);
        }

        // Store secret header if provided
        if let Some(h) = secret_header {
            self.secret_headers.write().await.insert(name.clone(), h);
        }

        // Store verification mode if provided
        if let Some(m) = verification_mode {
            self.verification_modes.write().await.insert(name, m);
        }
    }

    /// Get the secret header name for a channel.
    ///
    /// Returns the configured header or "X-Webhook-Secret" as default.
    pub async fn get_secret_header(&self, channel_name: &str) -> String {
        self.secret_headers
            .read()
            .await
            .get(channel_name)
            .cloned()
            .unwrap_or_else(|| "X-Webhook-Secret".to_string())
    }

    /// Get the verification mode for a channel.
    ///
    /// Returns the configured mode or None (default behavior).
    pub async fn get_verification_mode(&self, channel_name: &str) -> Option<String> {
        self.verification_modes
            .read()
            .await
            .get(channel_name)
            .cloned()
    }

    /// Update the webhook secret for an already-registered channel.
    ///
    /// This is used when credentials are saved after a channel was registered
    /// without a secret (e.g., loaded at startup before the user configured it).
    pub async fn update_secret(&self, channel_name: &str, secret: String) {
        self.secrets
            .write()
            .await
            .insert(channel_name.to_string(), secret);
        tracing::info!(
            channel = %channel_name,
            "Updated webhook secret for channel"
        );
    }

    /// Unregister a channel and its endpoints.
    pub async fn unregister(&self, channel_name: &str) {
        self.channels.write().await.remove(channel_name);
        self.secrets.write().await.remove(channel_name);
        self.secret_headers.write().await.remove(channel_name);
        self.signature_keys.write().await.remove(channel_name);
        self.verification_modes.write().await.remove(channel_name);
        self.hmac_secrets.write().await.remove(channel_name);

        // Remove all paths for this channel
        self.path_to_channel
            .write()
            .await
            .retain(|_, name| name != channel_name);

        tracing::info!(
            channel = %channel_name,
            "Unregistered WASM channel"
        );
    }

    /// Get the channel for a given path.
    pub async fn get_channel_for_path(&self, path: &str) -> Option<Arc<WasmChannel>> {
        let path_map = self.path_to_channel.read().await;
        let channel_name = path_map.get(path)?;

        self.channels.read().await.get(channel_name).cloned()
    }

    /// Validate a secret for a channel.
    pub async fn validate_secret(&self, channel_name: &str, provided: &str) -> bool {
        let secrets = self.secrets.read().await;
        match secrets.get(channel_name) {
            Some(expected) => expected == provided,
            None => true, // No secret required
        }
    }

    /// Check if a channel requires a secret.
    pub async fn requires_secret(&self, channel_name: &str) -> bool {
        self.secrets.read().await.contains_key(channel_name)
    }

    /// List all registered channels.
    pub async fn list_channels(&self) -> Vec<String> {
        self.channels.read().await.keys().cloned().collect()
    }

    /// List all registered paths.
    pub async fn list_paths(&self) -> Vec<String> {
        self.path_to_channel.read().await.keys().cloned().collect()
    }

    /// Register an Ed25519 public key for signature verification.
    ///
    /// Validates that the key is valid hex encoding of a 32-byte Ed25519 public key.
    /// Channels with a registered key will have Discord-style Ed25519
    /// signature validation performed before forwarding to WASM.
    pub async fn register_signature_key(
        &self,
        channel_name: &str,
        public_key_hex: &str,
    ) -> Result<(), String> {
        use ed25519_dalek::VerifyingKey;

        let key_bytes = hex::decode(public_key_hex).map_err(|e| format!("invalid hex: {e}"))?;
        VerifyingKey::try_from(key_bytes.as_slice())
            .map_err(|e| format!("invalid Ed25519 public key: {e}"))?;

        self.signature_keys
            .write()
            .await
            .insert(channel_name.to_string(), public_key_hex.to_string());
        Ok(())
    }

    /// Get the signature verification key for a channel.
    ///
    /// Returns `None` if no key is registered (no signature check needed).
    pub async fn get_signature_key(&self, channel_name: &str) -> Option<String> {
        self.signature_keys.read().await.get(channel_name).cloned()
    }

    /// Register an HMAC secret for a channel.
    ///
    /// The secret is used for HMAC-SHA256 signature verification
    /// (WhatsApp/Slack style with X-Hub-Signature-256 header).
    pub async fn register_hmac_secret(&self, channel_name: &str, secret: String) {
        self.hmac_secrets
            .write()
            .await
            .insert(channel_name.to_string(), secret);
        tracing::info!(
            channel = %channel_name,
            "Registered HMAC secret for webhook signature verification"
        );
    }

    /// Get the HMAC secret for a channel.
    ///
    /// Returns `None` if no HMAC secret is registered.
    pub async fn get_hmac_secret(&self, channel_name: &str) -> Option<String> {
        self.hmac_secrets.read().await.get(channel_name).cloned()
    }

    // ========================================================================
    // Webhook Acknowledgment (reliable message processing)
    // ========================================================================

    /// Register a pending acknowledgment for a webhook message.
    ///
    /// Call this before processing a webhook message. The returned receiver
    /// will be signaled when the message has been persisted to the database.
    /// The webhook handler should wait on this receiver before returning 200 OK.
    ///
    /// # Arguments
    /// * `key` - Unique identifier for the message, typically "channel:message_id"
    ///
    /// # Returns
    /// A oneshot receiver that will be signaled when ack_message() is called.
    pub async fn register_pending_ack(&self, key: String) -> oneshot::Receiver<()> {
        let (tx, rx) = oneshot::channel();
        self.pending_acks.write().await.insert(key.clone(), tx);
        tracing::debug!(key = %key, "Registered pending webhook ACK");
        rx
    }

    /// Signal that a message has been persisted and the webhook can return 200 OK.
    ///
    /// Called by the agent loop after persist_user_message() completes.
    /// Also triggers mark_as_read for channels that support it (WhatsApp).
    ///
    /// Note: Deduplication recording happens at webhook handler level (before
    /// sending to agent) to prevent race conditions with concurrent webhooks.
    ///
    /// # Arguments
    /// * `key` - The same key passed to register_pending_ack()
    /// * `message_metadata` - Optional JSON metadata for mark_as_read (phone_number_id, etc.)
    pub async fn ack_message(&self, key: &str, message_metadata: Option<&str>) {
        if let Some(tx) = self.pending_acks.write().await.remove(key) {
            // Signal the webhook handler to return 200 OK
            let _ = tx.send(());
            tracing::debug!(key = %key, "Webhook ACK signaled");

            // Trigger mark_as_read for supported channels
            if let Some(metadata) = message_metadata
                && let Err(e) = self.trigger_mark_as_read(key, metadata).await
            {
                tracing::warn!(key = %key, error = %e, "Failed to mark message as read");
            }
        } else {
            tracing::debug!(key = %key, "No pending ACK found (may have timed out)");
        }
    }

    /// Store access token for a channel (for host-side API calls like mark_as_read).
    pub async fn register_access_token(&self, channel_name: &str, token: String) {
        self.access_tokens
            .write()
            .await
            .insert(channel_name.to_string(), token);
        tracing::info!(channel = %channel_name, "Registered access token for mark_as_read");
    }

    /// Store API version for a channel (for host-side API calls).
    pub async fn register_api_version(&self, channel_name: &str, version: String) {
        self.api_versions
            .write()
            .await
            .insert(channel_name.to_string(), version);
    }

    /// Get the access token for a channel.
    pub async fn get_access_token(&self, channel_name: &str) -> Option<String> {
        self.access_tokens.read().await.get(channel_name).cloned()
    }

    /// Get the API version for a channel.
    pub async fn get_api_version(&self, channel_name: &str) -> Option<String> {
        self.api_versions.read().await.get(channel_name).cloned()
    }

    /// Trigger mark_as_read for a message after it's been persisted.
    ///
    /// Parses the key to extract channel name and calls the appropriate API.
    async fn trigger_mark_as_read(&self, key: &str, metadata_json: &str) -> Result<(), String> {
        // Parse key format: "channel:message_id"
        let parts: Vec<&str> = key.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(format!("Invalid ACK key format: {}", key));
        }
        let channel_name = parts[0];
        let message_id = parts[1];

        // Only WhatsApp supports mark_as_read currently
        if channel_name != "whatsapp" {
            tracing::debug!(channel = %channel_name, "Channel does not support mark_as_read");
            return Ok(());
        }

        // Parse metadata to get phone_number_id
        let metadata: WhatsAppMetadata =
            serde_json::from_str(metadata_json).map_err(|e| format!("Invalid metadata: {}", e))?;

        let phone_number_id = metadata
            .phone_number_id
            .ok_or_else(|| "Missing phone_number_id in metadata".to_string())?;

        // Get stored credentials
        let access_token = self
            .get_access_token(channel_name)
            .await
            .ok_or_else(|| "No access token registered for WhatsApp".to_string())?;
        let api_version = self
            .get_api_version(channel_name)
            .await
            .unwrap_or_else(|| "v25.0".to_string());

        // Call WhatsApp API to mark as read
        let url = format!(
            "https://graph.facebook.com/{}/{}/messages",
            api_version, phone_number_id
        );
        let payload = serde_json::json!({
            "messaging_product": "whatsapp",
            "status": "read",
            "message_id": message_id
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", access_token))
            .json(&payload)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        if response.status().is_success() {
            tracing::debug!(message_id = %message_id, "Marked WhatsApp message as read");
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(format!("WhatsApp API error: {} - {}", status, body))
        }
    }
}

impl Default for WasmChannelRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared state for the HTTP server.
#[allow(dead_code)]
#[derive(Clone)]
pub struct RouterState {
    router: Arc<WasmChannelRouter>,
    extension_manager: Option<Arc<crate::extensions::ExtensionManager>>,
    /// Database for webhook message deduplication.
    db: Option<Arc<dyn crate::db::Database>>,
    /// Timeout for waiting on webhook ACK before returning 500.
    webhook_ack_timeout: Duration,
}

impl RouterState {
    pub fn new(router: Arc<WasmChannelRouter>) -> Self {
        Self {
            router,
            extension_manager: None,
            db: None,
            webhook_ack_timeout: Duration::from_secs(10),
        }
    }

    pub fn with_extension_manager(
        mut self,
        manager: Arc<crate::extensions::ExtensionManager>,
    ) -> Self {
        self.extension_manager = Some(manager);
        self
    }

    /// Add database for webhook message deduplication.
    pub fn with_db(mut self, db: Arc<dyn crate::db::Database>) -> Self {
        self.db = Some(db);
        self
    }

    /// Set the webhook ACK timeout.
    pub fn with_webhook_ack_timeout(mut self, timeout: Duration) -> Self {
        self.webhook_ack_timeout = timeout;
        self
    }
}

/// Webhook request body for WASM channels.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct WasmWebhookRequest {
    /// Optional secret for authentication.
    #[serde(default)]
    pub secret: Option<String>,
}

/// Health response.
#[allow(dead_code)]
#[derive(Debug, Serialize)]
struct HealthResponse {
    status: String,
    channels: Vec<String>,
}

/// Handler for health check endpoint.
#[allow(dead_code)]
async fn health_handler(State(state): State<RouterState>) -> impl IntoResponse {
    let channels = state.router.list_channels().await;
    Json(HealthResponse {
        status: "healthy".to_string(),
        channels,
    })
}

/// Generic webhook handler that routes to the appropriate WASM channel.
async fn webhook_handler(
    State(state): State<RouterState>,
    method: Method,
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let full_path = format!("/webhook/{}", path);

    tracing::info!(
        method = %method,
        path = %full_path,
        body_len = body.len(),
        "Webhook request received"
    );

    // Find the channel for this path
    let channel = match state.router.get_channel_for_path(&full_path).await {
        Some(c) => c,
        None => {
            tracing::warn!(
                path = %full_path,
                "No channel registered for webhook path"
            );
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "Channel not found for path",
                    "path": full_path
                })),
            );
        }
    };

    tracing::info!(
        channel = %channel.channel_name(),
        "Found channel for webhook"
    );

    let channel_name = channel.channel_name();

    // Check if secret is required
    // Skip secret validation if using query_param verification mode
    // (e.g., WhatsApp uses hub.verify_token for GET verification and X-Hub-Signature-256
    // for POST validation - both handled by the WASM module internally)
    let verification_mode = state.router.get_verification_mode(channel_name).await;
    let skip_secret_validation = verification_mode.as_deref() == Some("query_param");

    if !skip_secret_validation && state.router.requires_secret(channel_name).await {
        // Get the secret header name for this channel (from capabilities or default)
        let secret_header_name = state.router.get_secret_header(channel_name).await;

        // Try to get secret from query param or the channel's configured header
        let provided_secret = query
            .get("secret")
            .cloned()
            .or_else(|| {
                headers
                    .get(&secret_header_name)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string())
            })
            .or_else(|| {
                // Fallback to generic header if different from configured
                if secret_header_name != "X-Webhook-Secret" {
                    headers
                        .get("X-Webhook-Secret")
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            });

        tracing::debug!(
            channel = %channel_name,
            has_provided_secret = provided_secret.is_some(),
            provided_secret_len = provided_secret.as_ref().map(|s| s.len()),
            "Checking webhook secret"
        );

        match provided_secret {
            Some(secret) => {
                if !state.router.validate_secret(channel_name, &secret).await {
                    tracing::warn!(
                        channel = %channel_name,
                        "Webhook secret validation failed"
                    );
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({
                            "error": "Invalid webhook secret"
                        })),
                    );
                }
                tracing::debug!(channel = %channel_name, "Webhook secret validated");
            }
            None => {
                tracing::warn!(
                    channel = %channel_name,
                    "Webhook secret required but not provided"
                );
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": "Webhook secret required"
                    })),
                );
            }
        }
    } else if skip_secret_validation {
        tracing::debug!(
            channel = %channel_name,
            verification_mode = ?verification_mode,
            "Skipping secret validation for channel with query_param verification mode"
        );
    }

    // Ed25519 signature verification (Discord-style)
    if let Some(pub_key_hex) = state.router.get_signature_key(channel_name).await {
        let sig_hex = headers
            .get("x-signature-ed25519")
            .and_then(|v| v.to_str().ok());
        let timestamp = headers
            .get("x-signature-timestamp")
            .and_then(|v| v.to_str().ok());

        match (sig_hex, timestamp) {
            (Some(sig), Some(ts)) => {
                let now_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;

                if !crate::channels::wasm::signature::verify_discord_signature(
                    &pub_key_hex,
                    sig,
                    ts,
                    &body,
                    now_secs,
                ) {
                    tracing::warn!(
                        channel = %channel_name,
                        "Ed25519 signature verification failed"
                    );
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({
                            "error": "Invalid signature"
                        })),
                    );
                }
                tracing::debug!(channel = %channel_name, "Ed25519 signature verified");
            }
            _ => {
                tracing::warn!(
                    channel = %channel_name,
                    "Signature headers missing but key is registered"
                );
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": "Missing signature headers"
                    })),
                );
            }
        }
    }

    // HMAC-SHA256 signature verification (WhatsApp/Slack style)
    if let Some(hmac_secret) = state.router.get_hmac_secret(channel_name).await {
        let signature_header = headers
            .get("X-Hub-Signature-256")
            .and_then(|v| v.to_str().ok());

        match signature_header {
            Some(sig) => {
                if !crate::channels::wasm::signature::verify_hmac_sha256(&hmac_secret, sig, &body) {
                    tracing::warn!(
                        channel = %channel_name,
                        "HMAC-SHA256 signature verification failed"
                    );
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({
                            "error": "Invalid signature"
                        })),
                    );
                }
                tracing::debug!(channel = %channel_name, "HMAC-SHA256 signature verified");
            }
            None => {
                tracing::warn!(
                    channel = %channel_name,
                    "X-Hub-Signature-256 header missing but HMAC secret is registered"
                );
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": "Missing signature header"
                    })),
                );
            }
        }
    }

    // Convert headers to HashMap
    let headers_map: HashMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|v| (k.as_str().to_string(), v.to_string()))
        })
        .collect();

    // Call the WASM channel (with messages returned for ACK coordination)
    let secret_validated = state.router.requires_secret(channel_name).await;

    tracing::info!(
        channel = %channel_name,
        secret_validated = secret_validated,
        "Calling WASM channel on_http_request_with_messages"
    );

    match channel
        .call_on_http_request_with_messages(
            method.as_str(),
            &full_path,
            &headers_map,
            &query,
            &body,
            secret_validated,
        )
        .await
    {
        Ok(HttpResponseWithMessages {
            response,
            emitted_messages,
        }) => {
            let status =
                StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

            tracing::info!(
                channel = %channel_name,
                status = %status,
                body_len = response.body.len(),
                emitted_count = emitted_messages.len(),
                "WASM channel on_http_request completed"
            );

            // If there are emitted messages, wait for ACK before returning
            if !emitted_messages.is_empty() {
                // Register ACK receivers for each NEW message (skip duplicates)
                let mut ack_receivers: Vec<(String, oneshot::Receiver<()>, String)> = Vec::new();
                let mut new_messages: Vec<_> = Vec::new();

                for msg in &emitted_messages {
                    // Parse metadata to extract message_id for ACK key and deduplication
                    // For WhatsApp, metadata contains: phone_number_id, sender_phone, message_id, timestamp
                    let (ack_key, external_msg_id) = if let Ok(meta) =
                        serde_json::from_str::<WhatsAppMetadata>(&msg.metadata_json)
                    {
                        if let Some(msg_id) = &meta.message_id {
                            (format!("{}:{}", channel_name, msg_id), Some(msg_id.clone()))
                        } else {
                            // Fallback to user_id if no message_id
                            (format!("{}:{}", channel_name, msg.user_id), None)
                        }
                    } else {
                        (format!("{}:{}", channel_name, msg.user_id), None)
                    };

                    // Check for duplicate messages if database is available
                    // Record immediately after check to prevent race conditions
                    if let Some(ref db) = state.db
                        && let Some(msg_id) = &external_msg_id
                    {
                        match db.is_webhook_message_processed(channel_name, msg_id).await {
                            Ok(is_processed) => {
                                if is_processed {
                                    tracing::info!(
                                        channel = %channel_name,
                                        message_id = %msg_id,
                                        "Duplicate webhook message detected, skipping"
                                    );
                                    continue; // Skip this duplicate message
                                }
                                // Not a duplicate - record immediately to prevent races
                                // This ensures concurrent webhooks for the same message will be caught
                                if let Err(e) = db
                                    .record_webhook_message_processed(channel_name, msg_id)
                                    .await
                                {
                                    tracing::warn!(
                                        channel = %channel_name,
                                        message_id = %msg_id,
                                        error = %e,
                                        "Failed to record message as processed, dedup may not work on retry"
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    channel = %channel_name,
                                    message_id = %msg_id,
                                    error = %e,
                                    "Failed to check dedup, proceeding anyway"
                                );
                                // Continue processing on error (fail open)
                            }
                        }
                    }

                    let ack_rx = state.router.register_pending_ack(ack_key.clone()).await;
                    ack_receivers.push((ack_key, ack_rx, msg.metadata_json.clone()));
                    new_messages.push(msg.clone());
                }

                // If all messages were duplicates, return success immediately
                if new_messages.is_empty() {
                    tracing::info!(
                        channel = %channel_name,
                        "All webhook messages were duplicates, returning 200"
                    );
                    let body_json: serde_json::Value = serde_json::from_slice(&response.body)
                        .unwrap_or_else(|_| {
                            serde_json::json!({
                                "raw": String::from_utf8_lossy(&response.body).to_string()
                            })
                        });
                    return (status, Json(body_json));
                }

                // Send only NEW messages to the agent
                if let Err(e) = channel.send_emitted_messages(new_messages).await {
                    tracing::error!(
                        channel = %channel_name,
                        error = %e,
                        "Failed to send emitted messages to agent"
                    );
                    // Continue to return response even if sending failed
                }

                // Wait for all ACKs with a configurable timeout for all messages
                // Using join_all ensures we don't accumulate timeouts (3 messages ≠ 30s wait)
                let ack_timeout = state.webhook_ack_timeout;

                let ack_futures: Vec<_> = ack_receivers
                    .into_iter()
                    .map(|(ack_key, ack_rx, _)| async move {
                        match ack_rx.await {
                            Ok(()) => {
                                tracing::debug!(key = %ack_key, "Webhook ACK received");
                                true
                            }
                            Err(_) => {
                                tracing::warn!(key = %ack_key, "Webhook ACK channel closed");
                                false
                            }
                        }
                    })
                    .collect();

                let all_acked =
                    match tokio::time::timeout(ack_timeout, futures::future::join_all(ack_futures))
                        .await
                    {
                        Ok(results) => results.iter().all(|&r| r),
                        Err(_) => {
                            tracing::warn!(
                                channel = %channel_name,
                                timeout_secs = ack_timeout.as_secs(),
                                "Webhook ACK wait timed out, returning 500 to trigger retry"
                            );
                            // Return 500 so WhatsApp retries - deduplication will handle duplicates
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(serde_json::json!({
                                    "error": "ACK timeout",
                                    "message": "Message processing timed out, will retry"
                                })),
                            );
                        }
                    };

                if !all_acked {
                    tracing::warn!(
                        channel = %channel_name,
                        "Some webhook ACKs were not received, returning 500 to trigger retry"
                    );
                    // Return 500 so WhatsApp retries - deduplication will handle duplicates
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": "ACK not received",
                            "message": "Message persistence not confirmed, will retry"
                        })),
                    );
                }
            }

            // Build response with headers
            let body_json: serde_json::Value = serde_json::from_slice(&response.body)
                .unwrap_or_else(|_| {
                    serde_json::json!({
                        "raw": String::from_utf8_lossy(&response.body).to_string()
                    })
                });

            (status, Json(body_json))
        }
        Err(e) => {
            tracing::error!(
                channel = %channel_name,
                error = %e,
                "WASM channel callback failed"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Channel callback failed",
                    "details": e.to_string()
                })),
            )
        }
    }
}

/// OAuth callback handler for extension authentication.
///
/// Handles OAuth redirect callbacks at /oauth/callback?code=xxx&state=yyy.
/// This is used when authenticating MCP servers or WASM tool OAuth flows
/// via a tunnel URL (remote callback).
#[allow(dead_code)]
async fn oauth_callback_handler(
    State(_state): State<RouterState>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let code = params.get("code").cloned().unwrap_or_default();
    let _state = params.get("state").cloned().unwrap_or_default();

    if code.is_empty() {
        let error = params
            .get("error")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        return (
            StatusCode::BAD_REQUEST,
            axum::response::Html(format!(
                "<!DOCTYPE html><html><body style=\"font-family: sans-serif; \
                 display: flex; justify-content: center; align-items: center; \
                 height: 100vh; margin: 0; background: #191919; color: white;\">\
                 <div style=\"text-align: center;\">\
                 <h1>Authorization Failed</h1>\
                 <p>Error: {}</p>\
                 </div></body></html>",
                error
            )),
        );
    }

    // TODO: In a future iteration, use the state nonce to look up the pending auth
    // and complete the token exchange. For now, the OAuth flow uses local callbacks
    // via authorize_mcp_server() which handles the full flow synchronously.

    (
        StatusCode::OK,
        axum::response::Html(
            "<!DOCTYPE html><html><body style=\"font-family: sans-serif; \
             display: flex; justify-content: center; align-items: center; \
             height: 100vh; margin: 0; background: #191919; color: white;\">\
             <div style=\"text-align: center;\">\
             <h1>Connected!</h1>\
             <p>You can close this window and return to IronClaw.</p>\
             </div></body></html>"
                .to_string(),
        ),
    )
}

/// Create an Axum router for WASM channel webhooks.
///
/// This router can be merged with the existing HTTP channel router.
pub fn create_wasm_channel_router(
    router: Arc<WasmChannelRouter>,
    extension_manager: Option<Arc<crate::extensions::ExtensionManager>>,
    db: Option<Arc<dyn crate::db::Database>>,
    webhook_ack_timeout: Option<Duration>,
) -> Router {
    let mut state = RouterState::new(router);
    if let Some(manager) = extension_manager {
        state = state.with_extension_manager(manager);
    }
    if let Some(database) = db {
        state = state.with_db(database);
    }
    if let Some(timeout) = webhook_ack_timeout {
        state = state.with_webhook_ack_timeout(timeout);
    }

    Router::new()
        .route("/wasm-channels/health", get(health_handler))
        .route("/oauth/callback", get(oauth_callback_handler))
        // Catch-all for webhook paths
        .route("/webhook/{*path}", get(webhook_handler))
        .route("/webhook/{*path}", post(webhook_handler))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::channels::wasm::capabilities::ChannelCapabilities;
    use crate::channels::wasm::router::{RegisteredEndpoint, WasmChannelRouter};
    use crate::channels::wasm::runtime::{
        PreparedChannelModule, WasmChannelRuntime, WasmChannelRuntimeConfig,
    };
    use crate::channels::wasm::wrapper::WasmChannel;
    use crate::pairing::PairingStore;
    use crate::tools::wasm::ResourceLimits;

    fn create_test_channel(name: &str) -> Arc<WasmChannel> {
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

        let prepared = Arc::new(PreparedChannelModule {
            name: name.to_string(),
            description: format!("Test channel: {}", name),
            component: None,
            limits: ResourceLimits::default(),
        });

        let capabilities =
            ChannelCapabilities::for_channel(name).with_path(format!("/webhook/{}", name));

        Arc::new(WasmChannel::new(
            runtime,
            prepared,
            capabilities,
            "{}".to_string(),
            Arc::new(PairingStore::new()),
            None,
        ))
    }

    #[tokio::test]
    async fn test_router_register_and_lookup() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("slack");

        let endpoints = vec![RegisteredEndpoint {
            channel_name: "slack".to_string(),
            path: "/webhook/slack".to_string(),
            methods: vec!["POST".to_string()],
            require_secret: true,
        }];

        router
            .register(
                channel,
                endpoints,
                Some("secret123".to_string()),
                None,
                None,
            )
            .await;

        // Should find channel by path
        let found = router.get_channel_for_path("/webhook/slack").await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().channel_name(), "slack");

        // Should not find non-existent path
        let not_found = router.get_channel_for_path("/webhook/telegram").await;
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_router_secret_validation() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("slack");

        router
            .register(channel, vec![], Some("secret123".to_string()), None, None)
            .await;

        // Correct secret
        assert!(router.validate_secret("slack", "secret123").await);

        // Wrong secret
        assert!(!router.validate_secret("slack", "wrong").await);

        // Channel without secret always validates
        let channel2 = create_test_channel("telegram");
        router.register(channel2, vec![], None, None, None).await;
        assert!(router.validate_secret("telegram", "anything").await);
    }

    #[tokio::test]
    async fn test_router_unregister() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("slack");

        let endpoints = vec![RegisteredEndpoint {
            channel_name: "slack".to_string(),
            path: "/webhook/slack".to_string(),
            methods: vec!["POST".to_string()],
            require_secret: false,
        }];

        router.register(channel, endpoints, None, None, None).await;

        // Should exist
        assert!(
            router
                .get_channel_for_path("/webhook/slack")
                .await
                .is_some()
        );

        // Unregister
        router.unregister("slack").await;

        // Should no longer exist
        assert!(
            router
                .get_channel_for_path("/webhook/slack")
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_router_list_channels() {
        let router = WasmChannelRouter::new();

        let channel1 = create_test_channel("slack");
        let channel2 = create_test_channel("telegram");

        router.register(channel1, vec![], None, None, None).await;
        router.register(channel2, vec![], None, None, None).await;

        let channels = router.list_channels().await;
        assert_eq!(channels.len(), 2);
        assert!(channels.contains(&"slack".to_string()));
        assert!(channels.contains(&"telegram".to_string()));
    }

    #[tokio::test]
    async fn test_router_secret_header() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("telegram");

        // Register with custom secret header
        router
            .register(
                channel,
                vec![],
                Some("secret123".to_string()),
                Some("X-Telegram-Bot-Api-Secret-Token".to_string()),
                None,
            )
            .await;

        // Should return the custom header
        assert_eq!(
            router.get_secret_header("telegram").await,
            "X-Telegram-Bot-Api-Secret-Token"
        );

        // Channel without custom header should use default
        let channel2 = create_test_channel("slack");
        router
            .register(channel2, vec![], Some("secret456".to_string()), None, None)
            .await;
        assert_eq!(router.get_secret_header("slack").await, "X-Webhook-Secret");
    }

    // ── Category 3: Router Signature Key Management ─────────────────────

    #[tokio::test]
    async fn test_register_and_get_signature_key() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("discord");

        router.register(channel, vec![], None, None, None).await;

        let fake_pub_key = "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2";
        router
            .register_signature_key("discord", fake_pub_key)
            .await
            .unwrap();

        let key = router.get_signature_key("discord").await;
        assert_eq!(key, Some(fake_pub_key.to_string()));
    }

    #[tokio::test]
    async fn test_no_signature_key_returns_none() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("slack");
        router.register(channel, vec![], None, None, None).await;

        // Slack has no signature key registered
        let key = router.get_signature_key("slack").await;
        assert!(key.is_none());
    }

    #[tokio::test]
    async fn test_unregister_removes_signature_key() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("discord");

        let endpoints = vec![RegisteredEndpoint {
            channel_name: "discord".to_string(),
            path: "/webhook/discord".to_string(),
            methods: vec!["POST".to_string()],
            require_secret: false,
        }];

        router.register(channel, endpoints, None, None, None).await;
        // Use a valid 32-byte Ed25519 key for this test
        let valid_key = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa3f4a18446b7e8c7ac6602";
        router
            .register_signature_key("discord", valid_key)
            .await
            .unwrap();

        // Key should exist
        assert!(router.get_signature_key("discord").await.is_some());

        // Unregister
        router.unregister("discord").await;

        // Key should be gone
        assert!(router.get_signature_key("discord").await.is_none());
    }

    // ── Key Validation Tests ──────────────────────────────────────────

    #[tokio::test]
    async fn test_register_valid_signature_key_succeeds() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("discord");
        router.register(channel, vec![], None, None, None).await;

        // Valid 32-byte Ed25519 public key (from test keypair)
        let valid_key = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa3f4a18446b7e8c7ac6602";
        let result = router.register_signature_key("discord", valid_key).await;
        assert!(result.is_ok(), "Valid Ed25519 key should be accepted");
    }

    #[tokio::test]
    async fn test_register_invalid_hex_key_fails() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("discord");
        router.register(channel, vec![], None, None, None).await;

        let result = router
            .register_signature_key("discord", "not-valid-hex-zzz")
            .await;
        assert!(result.is_err(), "Invalid hex should be rejected");
    }

    #[tokio::test]
    async fn test_register_wrong_length_key_fails() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("discord");
        router.register(channel, vec![], None, None, None).await;

        // 16 bytes instead of 32
        let short_key = hex::encode([0u8; 16]);
        let result = router.register_signature_key("discord", &short_key).await;
        assert!(result.is_err(), "Wrong-length key should be rejected");
    }

    #[tokio::test]
    async fn test_register_empty_key_fails() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("discord");
        router.register(channel, vec![], None, None, None).await;

        let result = router.register_signature_key("discord", "").await;
        assert!(result.is_err(), "Empty key should be rejected");
    }

    #[tokio::test]
    async fn test_valid_key_is_retrievable() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("discord");
        router.register(channel, vec![], None, None, None).await;

        let valid_key = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa3f4a18446b7e8c7ac6602";
        router
            .register_signature_key("discord", valid_key)
            .await
            .unwrap();

        let stored = router.get_signature_key("discord").await;
        assert_eq!(stored, Some(valid_key.to_string()));
    }

    #[tokio::test]
    async fn test_invalid_key_does_not_store() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("discord");
        router.register(channel, vec![], None, None, None).await;

        // Attempt to register invalid key
        let _ = router
            .register_signature_key("discord", "not-valid-hex")
            .await;

        // Should not have stored anything
        let stored = router.get_signature_key("discord").await;
        assert!(stored.is_none(), "Invalid key should not be stored");
    }

    // ── Webhook Handler Integration Tests ─────────────────────────────

    use axum::Router as AxumRouter;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::channels::wasm::router::create_wasm_channel_router;
    use ed25519_dalek::{Signer, SigningKey};

    /// Helper to create a router with a registered channel at /webhook/discord.
    async fn setup_discord_router() -> (Arc<WasmChannelRouter>, AxumRouter) {
        let wasm_router = Arc::new(WasmChannelRouter::new());
        let channel = create_test_channel("discord");

        let endpoints = vec![RegisteredEndpoint {
            channel_name: "discord".to_string(),
            path: "/webhook/discord".to_string(),
            methods: vec!["POST".to_string()],
            require_secret: false,
        }];

        wasm_router
            .register(channel, endpoints, None, None, None)
            .await;

        let app = create_wasm_channel_router(wasm_router.clone(), None, None, None);
        (wasm_router, app)
    }

    /// Helper: generate a test keypair.
    fn test_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
            0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
            0x1c, 0xae, 0x7f, 0x60,
        ])
    }

    #[tokio::test]
    async fn test_webhook_rejects_missing_sig_headers() {
        let (wasm_router, app) = setup_discord_router().await;

        // Register a signature key
        let signing_key = test_signing_key();
        let pub_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
        wasm_router
            .register_signature_key("discord", &pub_key_hex)
            .await
            .unwrap();

        // Send request without signature headers
        let req = Request::builder()
            .method("POST")
            .uri("/webhook/discord")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"type":1}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "Missing signature headers should return 401"
        );
    }

    #[tokio::test]
    async fn test_webhook_rejects_invalid_signature() {
        let (wasm_router, app) = setup_discord_router().await;

        let signing_key = test_signing_key();
        let pub_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
        wasm_router
            .register_signature_key("discord", &pub_key_hex)
            .await
            .unwrap();

        let req = Request::builder()
            .method("POST")
            .uri("/webhook/discord")
            .header("content-type", "application/json")
            .header("x-signature-ed25519", "deadbeefdeadbeef")
            .header("x-signature-timestamp", "1234567890")
            .body(Body::from(r#"{"type":1}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "Invalid signature should return 401"
        );
    }

    #[tokio::test]
    async fn test_webhook_accepts_valid_signature() {
        let (wasm_router, app) = setup_discord_router().await;

        let signing_key = test_signing_key();
        let pub_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
        wasm_router
            .register_signature_key("discord", &pub_key_hex)
            .await
            .unwrap();

        // Use current timestamp so staleness check passes
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let timestamp = now_secs.to_string();
        let body_bytes = br#"{"type":1}"#;

        let mut message = Vec::new();
        message.extend_from_slice(timestamp.as_bytes());
        message.extend_from_slice(body_bytes);
        let signature = signing_key.sign(&message);
        let sig_hex = hex::encode(signature.to_bytes());

        let req = Request::builder()
            .method("POST")
            .uri("/webhook/discord")
            .header("content-type", "application/json")
            .header("x-signature-ed25519", &sig_hex)
            .header("x-signature-timestamp", &timestamp)
            .body(Body::from(&body_bytes[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Should NOT be 401 — signature is valid (may be 500 since no WASM module)
        assert_ne!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "Valid signature should not return 401"
        );
    }

    #[tokio::test]
    async fn test_webhook_skips_sig_for_no_key() {
        let (_wasm_router, app) = setup_discord_router().await;

        // No signature key registered — should not require signature
        let req = Request::builder()
            .method("POST")
            .uri("/webhook/discord")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"type":1}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Should NOT be 401 (may be 500 since no WASM module, but not auth failure)
        assert_ne!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "No signature key registered — should skip sig check"
        );
    }

    #[tokio::test]
    async fn test_webhook_sig_check_uses_body() {
        let (wasm_router, app) = setup_discord_router().await;

        let signing_key = test_signing_key();
        let pub_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
        wasm_router
            .register_signature_key("discord", &pub_key_hex)
            .await
            .unwrap();

        let timestamp = "1234567890";
        // Sign body A
        let body_a = br#"{"type":1}"#;
        let mut message = Vec::new();
        message.extend_from_slice(timestamp.as_bytes());
        message.extend_from_slice(body_a);
        let signature = signing_key.sign(&message);
        let sig_hex = hex::encode(signature.to_bytes());

        // But send body B
        let body_b = br#"{"type":2}"#;
        let req = Request::builder()
            .method("POST")
            .uri("/webhook/discord")
            .header("content-type", "application/json")
            .header("x-signature-ed25519", &sig_hex)
            .header("x-signature-timestamp", timestamp)
            .body(Body::from(&body_b[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "Signature for different body should return 401"
        );
    }

    #[tokio::test]
    async fn test_webhook_sig_check_uses_timestamp() {
        let (wasm_router, app) = setup_discord_router().await;

        let signing_key = test_signing_key();
        let pub_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
        wasm_router
            .register_signature_key("discord", &pub_key_hex)
            .await
            .unwrap();

        // Sign with timestamp A
        let timestamp_a = "1234567890";
        let body = br#"{"type":1}"#;
        let mut message = Vec::new();
        message.extend_from_slice(timestamp_a.as_bytes());
        message.extend_from_slice(body);
        let signature = signing_key.sign(&message);
        let sig_hex = hex::encode(signature.to_bytes());

        // But send timestamp B in the header
        let timestamp_b = "9999999999";
        let req = Request::builder()
            .method("POST")
            .uri("/webhook/discord")
            .header("content-type", "application/json")
            .header("x-signature-ed25519", &sig_hex)
            .header("x-signature-timestamp", timestamp_b)
            .body(Body::from(&body[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "Signature with mismatched timestamp should return 401"
        );
    }

    #[tokio::test]
    async fn test_webhook_sig_plus_secret() {
        let wasm_router = Arc::new(WasmChannelRouter::new());
        let channel = create_test_channel("discord");

        let endpoints = vec![RegisteredEndpoint {
            channel_name: "discord".to_string(),
            path: "/webhook/discord".to_string(),
            methods: vec!["POST".to_string()],
            require_secret: true,
        }];

        // Register with BOTH secret and signature key
        wasm_router
            .register(
                channel,
                endpoints,
                Some("my-secret".to_string()),
                None,
                None,
            )
            .await;

        let signing_key = test_signing_key();
        let pub_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
        wasm_router
            .register_signature_key("discord", &pub_key_hex)
            .await
            .unwrap();

        let app = create_wasm_channel_router(wasm_router.clone(), None, None, None);

        // Use current timestamp so staleness check passes
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let timestamp = now_secs.to_string();
        let body = br#"{"type":1}"#;
        let mut message = Vec::new();
        message.extend_from_slice(timestamp.as_bytes());
        message.extend_from_slice(body);
        let signature = signing_key.sign(&message);
        let sig_hex = hex::encode(signature.to_bytes());

        // Provide valid signature AND valid secret
        let req = Request::builder()
            .method("POST")
            .uri("/webhook/discord?secret=my-secret")
            .header("content-type", "application/json")
            .header("x-signature-ed25519", &sig_hex)
            .header("x-signature-timestamp", &timestamp)
            .body(Body::from(&body[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Should pass both checks (may be 500 due to no WASM module, but not 401)
        assert_ne!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "Valid secret + valid signature should not return 401"
        );
    }

    // ── HMAC-SHA256 Router Tests (WhatsApp/Slack style) ─────────────────

    /// Helper: compute HMAC-SHA256 signature in WhatsApp format.
    fn compute_hmac_signature(secret: &str, body: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;

        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let result = mac.finalize();
        format!("sha256={}", hex::encode(result.into_bytes()))
    }

    #[tokio::test]
    async fn test_register_and_get_hmac_secret() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("whatsapp");

        router.register(channel, vec![], None, None, None).await;

        router
            .register_hmac_secret("whatsapp", "my_app_secret".to_string())
            .await;

        let secret = router.get_hmac_secret("whatsapp").await;
        assert_eq!(secret, Some("my_app_secret".to_string()));
    }

    #[tokio::test]
    async fn test_no_hmac_secret_returns_none() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("telegram");
        router.register(channel, vec![], None, None, None).await;

        let secret = router.get_hmac_secret("telegram").await;
        assert!(secret.is_none());
    }

    #[tokio::test]
    async fn test_unregister_removes_hmac_secret() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("whatsapp");

        let endpoints = vec![RegisteredEndpoint {
            channel_name: "whatsapp".to_string(),
            path: "/webhook/whatsapp".to_string(),
            methods: vec!["POST".to_string()],
            require_secret: false,
        }];

        router.register(channel, endpoints, None, None, None).await;
        router
            .register_hmac_secret("whatsapp", "secret123".to_string())
            .await;

        // Secret should exist
        assert!(router.get_hmac_secret("whatsapp").await.is_some());

        // Unregister
        router.unregister("whatsapp").await;

        // Secret should be gone
        assert!(router.get_hmac_secret("whatsapp").await.is_none());
    }

    #[tokio::test]
    async fn test_webhook_rejects_missing_hmac_header() {
        let wasm_router = Arc::new(WasmChannelRouter::new());
        let channel = create_test_channel("whatsapp");

        let endpoints = vec![RegisteredEndpoint {
            channel_name: "whatsapp".to_string(),
            path: "/webhook/whatsapp".to_string(),
            methods: vec!["POST".to_string()],
            require_secret: false,
        }];

        wasm_router
            .register(
                channel,
                endpoints,
                None,
                None,
                Some("query_param".to_string()),
            )
            .await;

        // Register HMAC secret
        wasm_router
            .register_hmac_secret("whatsapp", "my_app_secret".to_string())
            .await;

        let app = create_wasm_channel_router(wasm_router.clone(), None, None, None);

        // Send request without X-Hub-Signature-256 header
        let req = Request::builder()
            .method("POST")
            .uri("/webhook/whatsapp")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"entry":[]}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "Missing HMAC header should return 401"
        );
    }

    #[tokio::test]
    async fn test_webhook_rejects_invalid_hmac_signature() {
        let wasm_router = Arc::new(WasmChannelRouter::new());
        let channel = create_test_channel("whatsapp");

        let endpoints = vec![RegisteredEndpoint {
            channel_name: "whatsapp".to_string(),
            path: "/webhook/whatsapp".to_string(),
            methods: vec!["POST".to_string()],
            require_secret: false,
        }];

        wasm_router
            .register(
                channel,
                endpoints,
                None,
                None,
                Some("query_param".to_string()),
            )
            .await;

        wasm_router
            .register_hmac_secret("whatsapp", "correct_secret".to_string())
            .await;

        let app = create_wasm_channel_router(wasm_router.clone(), None, None, None);

        let body = br#"{"entry":[]}"#;
        // Sign with wrong secret
        let sig = compute_hmac_signature("wrong_secret", body);

        let req = Request::builder()
            .method("POST")
            .uri("/webhook/whatsapp")
            .header("content-type", "application/json")
            .header("X-Hub-Signature-256", &sig)
            .body(Body::from(&body[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "Invalid HMAC signature should return 401"
        );
    }

    #[tokio::test]
    async fn test_webhook_accepts_valid_hmac_signature() {
        let wasm_router = Arc::new(WasmChannelRouter::new());
        let channel = create_test_channel("whatsapp");

        let endpoints = vec![RegisteredEndpoint {
            channel_name: "whatsapp".to_string(),
            path: "/webhook/whatsapp".to_string(),
            methods: vec!["POST".to_string()],
            require_secret: false,
        }];

        wasm_router
            .register(
                channel,
                endpoints,
                None,
                None,
                Some("query_param".to_string()),
            )
            .await;

        wasm_router
            .register_hmac_secret("whatsapp", "my_app_secret".to_string())
            .await;

        let app = create_wasm_channel_router(wasm_router.clone(), None, None, None);

        let body = br#"{"entry":[]}"#;
        let sig = compute_hmac_signature("my_app_secret", body);

        let req = Request::builder()
            .method("POST")
            .uri("/webhook/whatsapp")
            .header("content-type", "application/json")
            .header("X-Hub-Signature-256", &sig)
            .body(Body::from(&body[..]))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Should pass HMAC check (may be 500 due to no WASM module, but not 401)
        assert_ne!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "Valid HMAC signature should not return 401"
        );
    }

    // ==================== Webhook ACK mechanism tests ====================

    #[tokio::test]
    async fn test_register_and_ack_message() {
        let router = WasmChannelRouter::new();

        // Register pending ACK
        let ack_key = "whatsapp:wamid.test123";
        let rx = router.register_pending_ack(ack_key.to_string()).await;

        // Ack the message
        router.ack_message(ack_key, None).await;

        // Verify the ACK was received
        let result = rx.await;
        assert!(result.is_ok(), "ACK should be received");
    }

    #[tokio::test]
    async fn test_ack_message_removes_pending_entry() {
        let router = WasmChannelRouter::new();

        let ack_key = "whatsapp:wamid.test456";
        let _rx = router.register_pending_ack(ack_key.to_string()).await;

        // Verify entry exists
        assert!(
            router.pending_acks.read().await.contains_key(ack_key),
            "Pending ACK should exist"
        );

        // Ack the message
        router.ack_message(ack_key, None).await;

        // Verify entry was removed
        assert!(
            !router.pending_acks.read().await.contains_key(ack_key),
            "Pending ACK should be removed after ACK"
        );
    }

    #[tokio::test]
    async fn test_ack_nonexistent_key_is_safe() {
        let router = WasmChannelRouter::new();

        // Should not panic when ACKing a key that was never registered
        router.ack_message("nonexistent:key", None).await;
    }

    #[tokio::test]
    async fn test_double_ack_same_key() {
        let router = WasmChannelRouter::new();

        let ack_key = "whatsapp:wamid.test789";
        let _rx = router.register_pending_ack(ack_key.to_string()).await;

        // First ACK
        router.ack_message(ack_key, None).await;

        // Second ACK should be safe (no panic)
        router.ack_message(ack_key, None).await;
    }
}
