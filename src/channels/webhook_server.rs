//! Unified HTTP server for all webhook routes.
//!
//! Composes route fragments from HttpChannel, WASM channel router, etc.
//! into a single axum server. Channels define routes but never spawn servers.

use std::net::SocketAddr;

use axum::Router;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::error::ChannelError;

/// Configuration for the unified webhook server.
pub struct WebhookServerConfig {
    /// Address to bind the server to.
    pub addr: SocketAddr,
}

/// A single HTTP server that hosts all webhook routes.
///
/// Channels contribute route fragments via `add_routes()`, then a single
/// `start()` call binds the listener and spawns the server task.
pub struct WebhookServer {
    config: WebhookServerConfig,
    routes: Vec<Router>,
    /// Merged router saved after start() for restart_with_addr().
    merged_router: Option<Router>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl WebhookServer {
    /// Create a new webhook server with the given bind address.
    pub fn new(config: WebhookServerConfig) -> Self {
        Self {
            config,
            routes: Vec::new(),
            merged_router: None,
            shutdown_tx: None,
            handle: None,
        }
    }

    /// Accumulate a route fragment. Each fragment should already have its
    /// state applied via `.with_state()`.
    pub fn add_routes(&mut self, router: Router) {
        self.routes.push(router);
    }

    /// Bind the listener, merge all route fragments, and spawn the server.
    pub async fn start(&mut self) -> Result<(), ChannelError> {
        let mut app = Router::new();
        for fragment in self.routes.drain(..) {
            app = app.merge(fragment);
        }
        self.merged_router = Some(app.clone());
        self.bind_and_spawn(app).await
    }

    /// Bind a listener to the configured address and spawn the server task.
    /// Private helper used by both start() and restart_with_addr().
    async fn bind_and_spawn(&mut self, app: Router) -> Result<(), ChannelError> {
        let listener = tokio::net::TcpListener::bind(self.config.addr)
            .await
            .map_err(|e| ChannelError::StartupFailed {
                name: "webhook_server".to_string(),
                reason: format!("Failed to bind to {}: {}", self.config.addr, e),
            })?;

        tracing::info!("Webhook server listening on {}", self.config.addr);

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                    tracing::info!("Webhook server shutting down");
                })
                .await
            {
                tracing::error!("Webhook server error: {}", e);
            }
        });

        self.handle = Some(handle);
        Ok(())
    }

    /// Gracefully shut down the current listener and rebind to a new address.
    /// The merged router from the original `start()` call is reused.
    pub async fn restart_with_addr(&mut self, new_addr: SocketAddr) -> Result<(), ChannelError> {
        self.shutdown().await;
        self.config.addr = new_addr;
        let app = self
            .merged_router
            .clone()
            .ok_or_else(|| ChannelError::StartupFailed {
                name: "webhook_server".to_string(),
                reason: "restart_with_addr called before start()".to_string(),
            })?;
        self.bind_and_spawn(app).await
    }

    /// Return the current bind address.
    pub fn current_addr(&self) -> SocketAddr {
        self.config.addr
    }

    /// Signal graceful shutdown and wait for the server task to finish.
    pub async fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use serde_json::json;

    #[tokio::test]
    async fn test_restart_with_addr_rebinds_listener() {
        // Start on port 0 (OS assigns after bind)
        let addr1 = "127.0.0.1:0".parse().unwrap();
        let mut server = WebhookServer::new(WebhookServerConfig { addr: addr1 });

        // Create a simple test router
        let test_router = axum::Router::new().route(
            "/test",
            axum::routing::get(|| async { Json(json!({"ok": true})) }),
        );
        server.add_routes(test_router);

        // Start the server
        server.start().await.expect("Failed to start server");

        // Allow a brief moment for the listener to bind and OS to assign port
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let old_addr = server.current_addr();
        // Note: port may still be 0 until OS actually binds, but the server is listening
        tracing::debug!("Server started on {}", old_addr);

        // Restart on a new addr (OS will assign new port)
        let addr2 = "127.0.0.1:0".parse().unwrap();
        server
            .restart_with_addr(addr2)
            .await
            .expect("Failed to restart with new addr");

        // Allow a brief moment for the new listener to bind
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let new_addr = server.current_addr();
        tracing::debug!("Server restarted on {}", new_addr);

        // Clean up
        server.shutdown().await;
    }
}
