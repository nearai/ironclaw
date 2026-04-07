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
    /// Merged router saved after start() for restarts via `install_listener()`.
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
    /// Private helper used by `start()`.
    async fn bind_and_spawn(&mut self, app: Router) -> Result<(), ChannelError> {
        let listener = tokio::net::TcpListener::bind(self.config.addr)
            .await
            .map_err(|e| ChannelError::StartupFailed {
                name: "webhook_server".to_string(),
                reason: format!("Failed to bind to {}: {}", self.config.addr, e),
            })?;

        tracing::debug!("Webhook server listening on {}", self.config.addr);

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                    tracing::debug!("Webhook server shutting down");
                })
                .await
            {
                tracing::error!("Webhook server error: {}", e);
            }
        });

        self.handle = Some(handle);
        Ok(())
    }

    /// Clone the merged router, if `start()` has been called.
    pub fn merged_router_clone(&self) -> Option<Router> {
        self.merged_router.clone()
    }

    /// Install a pre-bound listener, replacing the current one.
    ///
    /// The caller is responsible for binding the `TcpListener` *outside* any
    /// lock so that the async bind does not block other lock waiters. This
    /// method only does synchronous bookkeeping plus spawning the (non-blocking)
    /// server task, so it is safe to call while holding a mutex.
    pub fn install_listener(
        &mut self,
        new_addr: SocketAddr,
        listener: tokio::net::TcpListener,
        app: Router,
    ) -> (Option<oneshot::Sender<()>>, Option<JoinHandle<()>>) {
        // Capture old handles so the caller can shut them down outside the lock.
        let old_shutdown_tx = self.shutdown_tx.take();
        let old_handle = self.handle.take();

        self.config.addr = new_addr;

        // Spawn the new server task (non-blocking).
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                    tracing::debug!("Webhook server shutting down");
                })
                .await
            {
                tracing::error!("Webhook server error: {}", e);
            }
        });
        self.handle = Some(handle);

        tracing::debug!("Webhook server listening on {}", new_addr);

        (old_shutdown_tx, old_handle)
    }

    /// Return the current bind address.
    pub fn current_addr(&self) -> SocketAddr {
        self.config.addr
    }

    /// Take ownership of shutdown primitives so callers can perform async
    /// shutdown work without holding external locks around this server.
    pub fn begin_shutdown(&mut self) -> (Option<oneshot::Sender<()>>, Option<JoinHandle<()>>) {
        (self.shutdown_tx.take(), self.handle.take())
    }

    /// Signal graceful shutdown and wait for the server task to finish.
    pub async fn shutdown(&mut self) {
        let (shutdown_tx, handle) = self.begin_shutdown();
        if let Some(tx) = shutdown_tx {
            let _ = tx.send(());
        }
        if let Some(handle) = handle {
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
        use std::net::TcpListener as StdTcpListener;

        // Find two available ports by binding and immediately closing
        let port1 = {
            let listener =
                StdTcpListener::bind("127.0.0.1:0").expect("Failed to find available port 1");
            listener
                .local_addr()
                .expect("Failed to get local addr")
                .port()
        };

        let port2 = {
            let listener =
                StdTcpListener::bind("127.0.0.1:0").expect("Failed to find available port 2");
            listener
                .local_addr()
                .expect("Failed to get local addr")
                .port()
        };

        assert_ne!(port1, port2, "Should have different ports");
        assert_ne!(port1, 0, "Port 1 should be non-zero");
        assert_ne!(port2, 0, "Port 2 should be non-zero");

        // Start server on first port
        let addr1 = format!("127.0.0.1:{}", port1).parse().unwrap();
        let mut server = WebhookServer::new(WebhookServerConfig { addr: addr1 });

        // Create a test router that responds to health checks
        let test_router = axum::Router::new().route(
            "/health",
            axum::routing::get(|| async { Json(json!({"status": "ok"})) }),
        );
        server.add_routes(test_router);

        // Start the server on first port
        server.start().await.expect("Failed to start server");
        assert_eq!(
            server.current_addr(),
            addr1,
            "Server should be bound to initial address"
        );

        // Verify the first server is actually listening
        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://{}/health", addr1))
            .send()
            .await
            .expect("Failed to send request to first server");
        assert_eq!(
            response.status(),
            200,
            "First server should respond to health check"
        );

        // Restart on second port using two-phase approach
        let addr2: SocketAddr = format!("127.0.0.1:{}", port2).parse().unwrap();
        let app = server
            .merged_router_clone()
            .expect("Router should exist after start()");
        let listener = tokio::net::TcpListener::bind(addr2)
            .await
            .expect("Failed to bind to new addr");
        let (old_tx, old_handle) = server.install_listener(addr2, listener, app);
        if let Some(tx) = old_tx {
            let _ = tx.send(());
        }
        if let Some(handle) = old_handle {
            let _ = handle.await;
        }

        // Assert the address changed
        assert_eq!(
            server.current_addr(),
            addr2,
            "Server address should be updated after restart"
        );
        assert_ne!(
            addr1, addr2,
            "Address should change after restart_with_addr"
        );

        // Verify the new server is actually listening on the new address
        let response = client
            .get(format!("http://{}/health", addr2))
            .send()
            .await
            .expect("Failed to send request to restarted server");
        assert_eq!(
            response.status(),
            200,
            "Restarted server should respond to health check on new address"
        );

        // Verify the old address is no longer responding
        let old_result = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            client.get(format!("http://{}/health", addr1)).send(),
        )
        .await;
        assert!(
            old_result.is_err() || old_result.as_ref().unwrap().is_err(),
            "Old address should not respond after server restarts"
        );

        // Clean up
        server.shutdown().await;
    }

    #[tokio::test]
    async fn test_begin_shutdown_takes_handles_for_lock_free_shutdown() {
        let addr = SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 0));
        let mut server = WebhookServer::new(WebhookServerConfig { addr });

        let test_router = axum::Router::new().route(
            "/health",
            axum::routing::get(|| async { Json(json!({"status": "ok"})) }),
        );
        server.add_routes(test_router);
        server.start().await.expect("Failed to start server"); // safety: test assertion for setup precondition

        let (shutdown_tx, handle) = server.begin_shutdown();
        assert!(shutdown_tx.is_some(), "shutdown sender should be available"); // safety: test assertion for expected server state
        assert!(handle.is_some(), "server handle should be available"); // safety: test assertion for expected server state

        // begin_shutdown() should leave no handles behind on the server.
        let (shutdown_tx2, handle2) = server.begin_shutdown();
        assert!(shutdown_tx2.is_none(), "shutdown sender should be consumed"); // safety: test assertion for postcondition
        assert!(handle2.is_none(), "server handle should be consumed"); // safety: test assertion for postcondition

        if let Some(tx) = shutdown_tx {
            let _ = tx.send(());
        }
        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }

    #[tokio::test]
    async fn test_restart_with_addr_rollback_on_bind_failure() {
        use std::net::TcpListener as StdTcpListener;

        // Find an available port
        let port1 = {
            let listener =
                StdTcpListener::bind("127.0.0.1:0").expect("Failed to find available port");
            listener
                .local_addr()
                .expect("Failed to get local addr")
                .port()
        };

        // Start server on first port
        let addr1 = format!("127.0.0.1:{}", port1).parse().unwrap();
        let mut server = WebhookServer::new(WebhookServerConfig { addr: addr1 });

        // Create a test router
        let test_router = axum::Router::new().route(
            "/health",
            axum::routing::get(|| async { Json(json!({"status": "ok"})) }),
        );
        server.add_routes(test_router);

        // Start the server on first port
        server.start().await.expect("Failed to start server");

        // Verify the server is listening
        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://{}/health", addr1))
            .send()
            .await
            .expect("Failed to send request");
        assert_eq!(response.status(), 200, "Server should be listening");

        // Try to restart on the SAME address — it's already in use, so bind
        // must fail regardless of privilege level (unlike port 1, which
        // succeeds as root).
        let invalid_addr: SocketAddr = addr1;

        // Attempt bind (should fail); server state is untouched because we
        // never call install_listener on failure.
        let app = server
            .merged_router_clone()
            .expect("Router should exist after start()");
        let result = tokio::net::TcpListener::bind(invalid_addr).await;
        assert!(result.is_err(), "Bind to already-in-use port should fail");
        // `app` is dropped — server state unchanged (rollback by construction)
        drop(app);

        // Verify the old address is still responding (rollback succeeded)
        let response = client
            .get(format!("http://{}/health", addr1))
            .send()
            .await
            .expect("Failed to send request to old address");
        assert_eq!(
            response.status(),
            200,
            "Old listener should still be running after failed restart"
        );

        // Verify the server address is unchanged
        assert_eq!(
            server.current_addr(),
            addr1,
            "Server address should be restored after failed restart"
        );

        // Clean up
        server.shutdown().await;
    }
}
