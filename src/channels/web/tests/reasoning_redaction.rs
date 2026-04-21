//! Regression: the browser-facing web gateway must not broadcast internal
//! reasoning updates. Web users should only receive final responses and tool
//! lifecycle activity, not model planning or tool-selection rationale.

use crate::channels::channel::Channel;
use crate::channels::{StatusUpdate, ToolDecision};
use crate::channels::web::GatewayChannel;
use crate::channels::web::sse::DEFAULT_BROADCAST_BUFFER;
use crate::config::GatewayConfig;
use futures::StreamExt;

fn test_gateway() -> GatewayChannel {
    GatewayChannel::new(
        GatewayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            auth_token: Some("test-token".to_string()),
            max_connections: 100,
            broadcast_buffer: DEFAULT_BROADCAST_BUFFER,
            workspace_read_scopes: vec![],
            memory_layers: vec![],
            oidc: None,
        },
        "test-user".to_string(),
    )
}

#[tokio::test]
async fn gateway_send_status_does_not_broadcast_reasoning_updates() {
    let gw = test_gateway();
    let mut stream = gw
        .state
        .sse
        .subscribe_raw(Some("test-user".to_string()), false)
        .expect("subscribe should succeed");
    let metadata = serde_json::json!({
        "user_id": "test-user",
        "thread_id": "thread-123"
    });

    gw.send_status(
        StatusUpdate::ReasoningUpdate {
            narrative: "I should call web_fetch, then inspect the page, then summarize it."
                .to_string(),
            decisions: vec![ToolDecision {
                tool_name: "web_fetch".to_string(),
                rationale: "Need to inspect the source before answering.".to_string(),
            }],
        },
        &metadata,
    )
    .await
    .expect("reasoning update should not error");

    let next = tokio::time::timeout(std::time::Duration::from_millis(150), stream.next()).await;
    assert!(
        next.is_err(),
        "web SSE should not expose reasoning_update events to browser clients"
    );
}
