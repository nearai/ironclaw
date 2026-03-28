//! Regression tests: the gateway channel must never silently drop messages.
//!
//! Previously, `respond()` and `broadcast()` returned `Ok(())` when thread_id
//! was missing, making callers believe the message was delivered when it wasn't.
//! These tests ensure that missing routing info produces an explicit error.

use crate::channels::channel::{Channel, IncomingMessage, OutgoingResponse};
use crate::channels::web::GatewayChannel;
use crate::config::GatewayConfig;
use crate::error::ChannelError;

fn test_gateway() -> GatewayChannel {
    GatewayChannel::new(
        GatewayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            auth_token: Some("test-token".to_string()),
            workspace_read_scopes: vec![],
            memory_layers: vec![],
        },
        "test-user".to_string(),
    )
}

#[tokio::test]
async fn gateway_respond_without_thread_id_returns_error() {
    let gw = test_gateway();
    let msg = IncomingMessage::new("gateway", "test-user", "hello");
    // msg has no thread_id by default
    assert!(msg.thread_id.is_none());

    let response = OutgoingResponse::text("reply");
    let result = gw.respond(&msg, response).await;

    assert!(
        result.is_err(),
        "respond() must not silently succeed without thread_id"
    );
    assert!(
        matches!(result, Err(ChannelError::MissingRoutingTarget { .. })),
        "Expected MissingRoutingTarget, got: {:?}",
        result
    );
}

#[tokio::test]
async fn gateway_respond_with_thread_id_succeeds() {
    let gw = test_gateway();
    let mut msg = IncomingMessage::new("gateway", "test-user", "hello");
    msg.thread_id = Some("thread-123".to_string());

    let response = OutgoingResponse::text("reply");
    let result = gw.respond(&msg, response).await;

    assert!(
        result.is_ok(),
        "respond() should succeed with thread_id: {:?}",
        result
    );
}

#[tokio::test]
async fn gateway_broadcast_without_thread_id_returns_error() {
    let gw = test_gateway();
    let response = OutgoingResponse::text("notification");
    // response has no thread_id by default

    let result = gw.broadcast("test-user", response).await;

    assert!(
        result.is_err(),
        "broadcast() must not silently succeed without thread_id"
    );
    assert!(
        matches!(result, Err(ChannelError::MissingRoutingTarget { .. })),
        "Expected MissingRoutingTarget, got: {:?}",
        result
    );
}

#[tokio::test]
async fn gateway_broadcast_with_thread_id_succeeds() {
    let gw = test_gateway();
    let response = OutgoingResponse::text("notification").in_thread("thread-456".to_string());

    let result = gw.broadcast("test-user", response).await;

    assert!(
        result.is_ok(),
        "broadcast() should succeed with thread_id: {:?}",
        result
    );
}
