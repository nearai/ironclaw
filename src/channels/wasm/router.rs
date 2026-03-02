//! HTTP router for WASM channel webhooks.
//!
//! Routes incoming HTTP requests to the appropriate WASM channel based on
//! registered paths. Handles secret validation at the host level.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::channels::wasm::wrapper::WasmChannel;

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
    pub async fn register(
        &self,
        channel: Arc<WasmChannel>,
        endpoints: Vec<RegisteredEndpoint>,
        secret: Option<String>,
        secret_header: Option<String>,
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
            self.secret_headers.write().await.insert(name, h);
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
}

impl RouterState {
    pub fn new(router: Arc<WasmChannelRouter>) -> Self {
        Self {
            router,
            extension_manager: None,
        }
    }

    pub fn with_extension_manager(
        mut self,
        manager: Arc<crate::extensions::ExtensionManager>,
    ) -> Self {
        self.extension_manager = Some(manager);
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
    if state.router.requires_secret(channel_name).await {
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

    // Convert headers to HashMap
    let headers_map: HashMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|v| (k.as_str().to_string(), v.to_string()))
        })
        .collect();

    // Call the WASM channel
    let secret_validated = state.router.requires_secret(channel_name).await;

    tracing::info!(
        channel = %channel_name,
        secret_validated = secret_validated,
        "Calling WASM channel on_http_request"
    );

    match channel
        .call_on_http_request(
            method.as_str(),
            &full_path,
            &headers_map,
            &query,
            &body,
            secret_validated,
        )
        .await
    {
        Ok(response) => {
            let status =
                StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

            tracing::info!(
                channel = %channel_name,
                status = %status,
                body_len = response.body.len(),
                "WASM channel on_http_request completed successfully"
            );

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
) -> Router {
    let mut state = RouterState::new(router);
    if let Some(manager) = extension_manager {
        state = state.with_extension_manager(manager);
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
            .register(channel, endpoints, Some("secret123".to_string()), None)
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
            .register(channel, vec![], Some("secret123".to_string()), None)
            .await;

        // Correct secret
        assert!(router.validate_secret("slack", "secret123").await);

        // Wrong secret
        assert!(!router.validate_secret("slack", "wrong").await);

        // Channel without secret always validates
        let channel2 = create_test_channel("telegram");
        router.register(channel2, vec![], None, None).await;
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

        router.register(channel, endpoints, None, None).await;

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

        router.register(channel1, vec![], None, None).await;
        router.register(channel2, vec![], None, None).await;

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
            .register(channel2, vec![], Some("secret456".to_string()), None)
            .await;
        assert_eq!(router.get_secret_header("slack").await, "X-Webhook-Secret");
    }

    // ── Category 3: Router Signature Key Management ─────────────────────

    #[tokio::test]
    async fn test_register_and_get_signature_key() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("discord");

        router.register(channel, vec![], None, None).await;

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
        router.register(channel, vec![], None, None).await;

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

        router.register(channel, endpoints, None, None).await;
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
        router.register(channel, vec![], None, None).await;

        // Valid 32-byte Ed25519 public key (from test keypair)
        let valid_key = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa3f4a18446b7e8c7ac6602";
        let result = router.register_signature_key("discord", valid_key).await;
        assert!(result.is_ok(), "Valid Ed25519 key should be accepted");
    }

    #[tokio::test]
    async fn test_register_invalid_hex_key_fails() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("discord");
        router.register(channel, vec![], None, None).await;

        let result = router
            .register_signature_key("discord", "not-valid-hex-zzz")
            .await;
        assert!(result.is_err(), "Invalid hex should be rejected");
    }

    #[tokio::test]
    async fn test_register_wrong_length_key_fails() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("discord");
        router.register(channel, vec![], None, None).await;

        // 16 bytes instead of 32
        let short_key = hex::encode([0u8; 16]);
        let result = router.register_signature_key("discord", &short_key).await;
        assert!(result.is_err(), "Wrong-length key should be rejected");
    }

    #[tokio::test]
    async fn test_register_empty_key_fails() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("discord");
        router.register(channel, vec![], None, None).await;

        let result = router.register_signature_key("discord", "").await;
        assert!(result.is_err(), "Empty key should be rejected");
    }

    #[tokio::test]
    async fn test_valid_key_is_retrievable() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("discord");
        router.register(channel, vec![], None, None).await;

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
        router.register(channel, vec![], None, None).await;

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

        wasm_router.register(channel, endpoints, None, None).await;

        let app = create_wasm_channel_router(wasm_router.clone(), None);
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
            .register(channel, endpoints, Some("my-secret".to_string()), None)
            .await;

        let signing_key = test_signing_key();
        let pub_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
        wasm_router
            .register_signature_key("discord", &pub_key_hex)
            .await
            .unwrap();

        let app = create_wasm_channel_router(wasm_router.clone(), None);

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
}
