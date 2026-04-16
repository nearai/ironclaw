use crate::channels::channel::{Channel, IncomingMessage, OutgoingResponse, StatusUpdate};
use crate::channels::web::GatewayChannel;
use crate::channels::web::types::AppEvent;
use crate::config::GatewayConfig;

fn test_gateway() -> GatewayChannel {
    GatewayChannel::new(
        GatewayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            auth_token: Some("test-token".to_string()),
            workspace_read_scopes: vec![],
            memory_layers: vec![],
            oidc: None,
        },
        "test-user".to_string(),
    )
}

#[tokio::test]
async fn gateway_respond_in_workspace_scopes_event_to_user() {
    let gateway = test_gateway();
    let mut receiver = gateway.state.sse.sender().subscribe();

    let mut msg = IncomingMessage::new("gateway", "test-user", "hello");
    msg.thread_id = Some("thread-123".to_string());
    msg.workspace_id = Some("workspace-123".to_string());

    gateway
        .respond(&msg, OutgoingResponse::text("reply"))
        .await
        .expect("respond should succeed");

    let scoped = receiver.recv().await.expect("event");
    assert_eq!(scoped.user_id.as_deref(), Some("test-user"));
    assert_eq!(scoped.workspace_id.as_deref(), Some("workspace-123"));
    assert!(matches!(
        scoped.event,
        AppEvent::Response {
            ref content,
            ref thread_id,
        } if content == "reply" && thread_id == "thread-123"
    ));
}

#[tokio::test]
async fn gateway_send_status_in_workspace_scopes_event_to_user() {
    let gateway = test_gateway();
    let mut receiver = gateway.state.sse.sender().subscribe();

    gateway
        .send_status(
            StatusUpdate::StreamChunk("partial".to_string()),
            &serde_json::json!({
                "thread_id": "thread-123",
                "user_id": "test-user",
                "workspace_id": "workspace-123",
            }),
        )
        .await
        .expect("send_status should succeed");

    let scoped = receiver.recv().await.expect("event");
    assert_eq!(scoped.user_id.as_deref(), Some("test-user"));
    assert_eq!(scoped.workspace_id.as_deref(), Some("workspace-123"));
    assert!(matches!(
        scoped.event,
        AppEvent::StreamChunk {
            ref content,
            thread_id: Some(ref thread_id),
        } if content == "partial" && thread_id == "thread-123"
    ));
}

#[tokio::test]
async fn gateway_broadcast_in_workspace_scopes_event_to_user() {
    let gateway = test_gateway();
    let mut receiver = gateway.state.sse.sender().subscribe();

    let mut response = OutgoingResponse::text("reply").in_thread("thread-123");
    response.metadata = serde_json::json!({
        "user_id": "test-user",
        "workspace_id": "workspace-123",
    });

    gateway
        .broadcast("test-user", response)
        .await
        .expect("broadcast should succeed");

    let scoped = receiver.recv().await.expect("event");
    assert_eq!(scoped.user_id.as_deref(), Some("test-user"));
    assert_eq!(scoped.workspace_id.as_deref(), Some("workspace-123"));
    assert!(matches!(
        scoped.event,
        AppEvent::Response {
            ref content,
            ref thread_id,
        } if content == "reply" && thread_id == "thread-123"
    ));
}
