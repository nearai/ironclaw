//! HTTP router for WASM channel webhooks.
//!
//! Routes incoming HTTP requests to the appropriate WASM channel based on
//! registered paths. Handles secret validation at the host level.

use std::collections::HashMap;
use std::net::SocketAddr;
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
}

impl WasmChannelRouter {
    /// Create a new router.
    pub fn new() -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
            path_to_channel: RwLock::new(HashMap::new()),
            secrets: RwLock::new(HashMap::new()),
        }
    }

    /// Register a channel with its endpoints.
    pub async fn register(
        &self,
        channel: Arc<WasmChannel>,
        endpoints: Vec<RegisteredEndpoint>,
        secret: Option<String>,
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
            self.secrets.write().await.insert(name, s);
        }
    }

    /// Unregister a channel and its endpoints.
    pub async fn unregister(&self, channel_name: &str) {
        self.channels.write().await.remove(channel_name);
        self.secrets.write().await.remove(channel_name);

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
}

impl RouterState {
    pub fn new(router: Arc<WasmChannelRouter>) -> Self {
        Self { router }
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
#[allow(dead_code)]
async fn webhook_handler(
    State(state): State<RouterState>,
    method: Method,
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let full_path = format!("/webhook/{}", path);

    // Find the channel for this path
    let channel = match state.router.get_channel_for_path(&full_path).await {
        Some(c) => c,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "Channel not found for path",
                    "path": full_path
                })),
            );
        }
    };

    let channel_name = channel.channel_name();

    // Check if secret is required
    if state.router.requires_secret(channel_name).await {
        // Try to get secret from query param or header
        let provided_secret = query.get("secret").cloned().or_else(|| {
            headers
                .get("X-Webhook-Secret")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        });

        match provided_secret {
            Some(secret) => {
                if !state.router.validate_secret(channel_name, &secret).await {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({
                            "error": "Invalid webhook secret"
                        })),
                    );
                }
            }
            None => {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": "Webhook secret required"
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

/// Create an Axum router for WASM channel webhooks.
///
/// This router can be merged with the existing HTTP channel router.
#[allow(dead_code)]
pub fn create_wasm_channel_router(router: Arc<WasmChannelRouter>) -> Router {
    let state = RouterState::new(router);

    Router::new()
        .route("/wasm-channels/health", get(health_handler))
        // Catch-all for webhook paths
        .route("/webhook/*path", get(webhook_handler))
        .route("/webhook/*path", post(webhook_handler))
        .with_state(state)
}

/// HTTP server for WASM channel webhooks.
#[allow(dead_code)]
pub struct WasmChannelServer {
    router: Arc<WasmChannelRouter>,
}

#[allow(dead_code)]
impl WasmChannelServer {
    /// Create a new server.
    pub fn new(router: Arc<WasmChannelRouter>) -> Self {
        Self { router }
    }

    /// Start the HTTP server.
    ///
    /// Returns a handle that can be used to shut down the server.
    pub async fn start(
        &self,
        addr: SocketAddr,
    ) -> Result<tokio::task::JoinHandle<()>, std::io::Error> {
        let app = create_wasm_channel_router(self.router.clone());

        let listener = tokio::net::TcpListener::bind(addr).await?;

        tracing::info!(
            addr = %addr,
            "WASM channel HTTP server started"
        );

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("WASM channel HTTP server error: {}", e);
            }
        });

        Ok(handle)
    }
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
    use crate::tools::wasm::ResourceLimits;

    fn create_test_channel(name: &str) -> Arc<WasmChannel> {
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

        let prepared = Arc::new(PreparedChannelModule {
            name: name.to_string(),
            description: format!("Test channel: {}", name),
            component_bytes: Vec::new(),
            limits: ResourceLimits::default(),
        });

        let capabilities =
            ChannelCapabilities::for_channel(name).with_path(format!("/webhook/{}", name));

        Arc::new(WasmChannel::new(
            runtime,
            prepared,
            capabilities,
            "{}".to_string(),
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
            .register(channel, endpoints, Some("secret123".to_string()))
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
            .register(channel, vec![], Some("secret123".to_string()))
            .await;

        // Correct secret
        assert!(router.validate_secret("slack", "secret123").await);

        // Wrong secret
        assert!(!router.validate_secret("slack", "wrong").await);

        // Channel without secret always validates
        let channel2 = create_test_channel("telegram");
        router.register(channel2, vec![], None).await;
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

        router.register(channel, endpoints, None).await;

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

        router.register(channel1, vec![], None).await;
        router.register(channel2, vec![], None).await;

        let channels = router.list_channels().await;
        assert_eq!(channels.len(), 2);
        assert!(channels.contains(&"slack".to_string()));
        assert!(channels.contains(&"telegram".to_string()));
    }
}
