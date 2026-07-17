//! Caller-level test for `serve_webui_v2`: spin up the real serve
//! loop on a kernel-picked port, drive it with `reqwest`, then trigger
//! graceful shutdown and confirm the bind socket closes.
//!
//! The v2 gateway `Router` (auth / CORS / body-limit / rate-limit) is
//! tested separately in
//! `crates/ironclaw_reborn_composition/tests/webui_v2_serve.rs`. This
//! test focuses on the seam this crate owns — the listener-binding +
//! serve-loop + graceful-shutdown behaviour — by handing the ingress a
//! trivial axum `Router`.

use std::net::SocketAddr;
use std::time::Duration;

use axum::{Router, extract::ConnectInfo, routing::get};
use ironclaw_webui::{RebornWebuiServeOptions, deferred_webui_v2_startup_router, serve_webui_v2};
use tokio::sync::oneshot;

async fn build_test_router() -> Router {
    Router::new()
        .route("/ping", get(|| async { "pong" }))
        .route(
            "/peer",
            get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.ip().to_string() }),
        )
}

fn test_client() -> reqwest::Client {
    // macOS dev environments may inherit a system proxy (ClashX,
    // OrbStack, etc.) that 502s loopback URLs. Force no_proxy so the
    // test is hermetic regardless of the operator's shell env.
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("test reqwest client builds")
}

#[tokio::test]
async fn serve_webui_v2_binds_and_serves_until_graceful_shutdown() {
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (bound_tx, bound_rx) = oneshot::channel::<SocketAddr>();

    let router = build_test_router().await;
    let opts = RebornWebuiServeOptions {
        addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        router,
        shutdown: shutdown_rx,
        bound_addr_tx: Some(bound_tx),
    };

    let serve_handle = tokio::spawn(async move { serve_webui_v2(opts).await });

    // Wait until the serve loop reports the actual bound address.
    let bound = tokio::time::timeout(Duration::from_secs(2), bound_rx)
        .await
        .expect("bound_addr_tx must fire within 2s")
        .expect("serve loop must send a bound addr before serving");

    let url = format!("http://{bound}/ping");
    let response = test_client()
        .get(&url)
        .send()
        .await
        .expect("/ping request must succeed against the bound listener");
    assert_eq!(response.status().as_u16(), 200, "expected /ping → 200");
    let body = response.text().await.expect("body");
    assert_eq!(body, "pong", "handler must reach the bound serve loop");

    let peer_url = format!("http://{bound}/peer");
    let response = test_client()
        .get(&peer_url)
        .send()
        .await
        .expect("/peer request must succeed against the bound listener");
    assert_eq!(
        response.status().as_u16(),
        200,
        "ConnectInfo handler should be served"
    );
    let body = response.text().await.expect("peer body");
    assert_eq!(
        body, "127.0.0.1",
        "serve_webui_v2 must inject peer ConnectInfo from the TCP listener"
    );

    // Trigger graceful shutdown and confirm the serve future returns.
    shutdown_tx
        .send(())
        .expect("shutdown receiver must be open");
    let outcome = tokio::time::timeout(Duration::from_secs(2), serve_handle)
        .await
        .expect("serve loop must exit within 2s of graceful-shutdown signal")
        .expect("serve loop join handle must not panic");
    outcome.expect("serve loop must return Ok after graceful shutdown");
}

#[tokio::test]
async fn serve_webui_v2_returns_bind_error_when_address_unusable() {
    // 240.0.0.1 is in IANA-reserved future-use block — bind must fail
    // with a kernel error, which the ingress layer maps to the
    // `Bind { addr, source }` variant rather than panicking.
    let (_shutdown_tx, shutdown_rx) = oneshot::channel();
    let router = build_test_router().await;
    let opts = RebornWebuiServeOptions {
        addr: SocketAddr::from(([240, 0, 0, 1], 1)),
        router,
        shutdown: shutdown_rx,
        bound_addr_tx: None,
    };

    let result = serve_webui_v2(opts).await;
    assert!(
        result.is_err(),
        "binding to an unusable address must surface as Err, got Ok"
    );
}

#[tokio::test]
async fn serve_webui_v2_shuts_down_when_shutdown_sender_drops() {
    // Defensive: if the host code that owns the shutdown sender exits
    // without explicitly firing it, the serve loop should still
    // terminate (the `_ = shutdown.await` branch treats the closed
    // channel as a shutdown request). Without this contract, a host
    // bug could leave a listener pinned for the lifetime of the
    // process.
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let (bound_tx, bound_rx) = oneshot::channel::<SocketAddr>();

    let router = build_test_router().await;
    let opts = RebornWebuiServeOptions {
        addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        router,
        shutdown: shutdown_rx,
        bound_addr_tx: Some(bound_tx),
    };

    let serve_handle = tokio::spawn(async move { serve_webui_v2(opts).await });
    let _bound = tokio::time::timeout(Duration::from_secs(2), bound_rx)
        .await
        .expect("bound_addr_tx must fire")
        .expect("bound addr");

    drop(shutdown_tx);

    let outcome = tokio::time::timeout(Duration::from_secs(2), serve_handle)
        .await
        .expect("serve loop must exit within 2s of shutdown-sender drop")
        .expect("serve loop join handle must not panic");
    outcome.expect("serve loop must return Ok after shutdown-sender drop");
}

#[tokio::test]
async fn deferred_startup_router_serves_health_then_delegates_when_ready() {
    let (startup_router, ready_handle) = deferred_webui_v2_startup_router();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (bound_tx, bound_rx) = oneshot::channel::<SocketAddr>();

    let serve_handle = tokio::spawn(async move {
        serve_webui_v2(RebornWebuiServeOptions {
            addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            router: startup_router,
            shutdown: shutdown_rx,
            bound_addr_tx: Some(bound_tx),
        })
        .await
    });

    let bound = tokio::time::timeout(Duration::from_secs(2), bound_rx)
        .await
        .expect("bound_addr_tx must fire")
        .expect("bound addr");

    let health_response = test_client()
        .get(format!("http://{bound}/api/health"))
        .send()
        .await
        .expect("startup health request succeeds");
    assert_eq!(
        health_response.status(),
        http::StatusCode::OK,
        "startup health must pass before runtime assembly finishes"
    );

    let before_ready = test_client()
        .get(format!("http://{bound}/ping"))
        .send()
        .await
        .expect("pre-ready request succeeds");
    assert_eq!(
        before_ready.status(),
        http::StatusCode::SERVICE_UNAVAILABLE,
        "non-health routes must not accept traffic before the runtime router is ready"
    );

    let ready_router = build_test_router().await;
    ready_handle
        .publish_ready_router(ready_router)
        .expect("startup router should still be listening");

    let after_ready = test_client()
        .get(format!("http://{bound}/ping"))
        .send()
        .await
        .expect("ready request succeeds");
    assert_eq!(
        after_ready.status(),
        http::StatusCode::OK,
        "startup router must delegate to the ready WebUI router without rebinding"
    );
    let body = after_ready.text().await.expect("ready body");
    assert_eq!(
        body, "pong",
        "ready request should reach the published router"
    );

    shutdown_tx
        .send(())
        .expect("shutdown receiver must be open");
    tokio::time::timeout(Duration::from_secs(2), serve_handle)
        .await
        .expect("serve loop must exit within 2s of shutdown signal")
        .expect("serve loop join handle must not panic")
        .expect("serve loop must return Ok after shutdown");
}
