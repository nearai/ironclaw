//! Shared test-support helpers for the WebChat v2 OAuth route tests.
//!
//! These live in `tests/support/` (a non-test-binary subdirectory) so
//! every `tests/*_oauth_routes.rs` integration file can `mod support;`
//! and reuse the mock-server scaffolding instead of copying it. The
//! inline `#[cfg(test)]` unit modules under `src/` cannot share this —
//! they are a different compilation unit — so they keep their own
//! private copies by necessity.

use std::net::SocketAddr;

use serde::Serialize;

/// A verified/unverified email entry returned by a mock `/user/emails`
/// endpoint.
#[derive(Serialize, Clone)]
pub struct MockEmail {
    pub email: &'static str,
    pub verified: bool,
    pub primary: bool,
}

/// Aborts the spawned mock-server task when the test's binding goes out
/// of scope, so neither the task nor its `TcpListener` outlives the
/// test that created it.
pub struct AbortOnDrop(pub tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Bind an ephemeral loopback port, serve `router` on it, and return
/// the bound address plus a drop guard that aborts the server.
pub async fn spawn_router(router: axum::Router) -> (SocketAddr, AbortOnDrop) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (addr, AbortOnDrop(handle))
}
